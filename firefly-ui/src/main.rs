#![windows_subsystem = "windows"]

use std::process::{Child, Command};
use shared_memory::{Shmem, ShmemConf};
use firefly_ipc::{DrawCommand, Op};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Com::*,
        Graphics::{
            Direct2D::{
                Common::{ D2D1_COLOR_F, D2D_RECT_F, D2D_SIZE_U },
                *,
            },
            DirectWrite::*,
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleA,
        UI::WindowsAndMessaging::*,
    },
};

struct WindowState {
    core_process: Child,
    ui_to_core_shmem: Shmem,
    core_to_ui_shmem: Shmem,
    d2d_factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    render_target: Option<ID2D1HwndRenderTarget>,
    text_format: Option<IDWriteTextFormat>,
    black_brush: Option<ID2D1SolidColorBrush>,
    lines: Vec<String>,
}

// 辅助函数：创建绘图资源
fn create_graphics_resources(state: &mut WindowState, hwnd: HWND) -> Result<()> {
    if state.render_target.is_some() {
        return Ok(());
    }

    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect)? };

    let render_target: ID2D1HwndRenderTarget = unsafe {
        let props = D2D1_RENDER_TARGET_PROPERTIES::default();
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U {
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            },
            ..Default::default()
        };
        state.d2d_factory.CreateHwndRenderTarget(&props, &hwnd_props)?
    };

    // ---- 这是最关键的修正之一：创建画刷时也要先转换接口 ----
    let black_brush: ID2D1SolidColorBrush = unsafe {
    let rt: &ID2D1RenderTarget = &render_target.cast()?;
    rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)?
	};
    // ---------------------------------------------------
    
    let text_format = unsafe {
        state.dwrite_factory.CreateTextFormat(
            w!("Consolas"), None,
            DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL,
            15.0, w!("en-us"),
        )?
    };

    state.render_target = Some(render_target);
    state.black_brush = Some(black_brush);
    state.text_format = Some(text_format);

    Ok(())
}

fn main() -> Result<()> {
    // ... main 函数的前半部分保持不变 ...
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED)? };
    let d2d_factory: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
    let dwrite_factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let ui_to_core_channel_id = "firefly_ui_to_core_channel";
    let core_to_ui_channel_id = "firefly_core_to_ui_channel";
    let ui_to_core_shmem = ShmemConf::new().size(4096).os_id(ui_to_core_channel_id).create().expect("UI: create ui_to_core failed");
    let core_to_ui_shmem = ShmemConf::new().size(4096).os_id(core_to_ui_channel_id).create().expect("UI: create core_to_ui failed");
    let mut core_path = std::env::current_exe().expect("UI: get current exe failed");
    core_path.pop();
    core_path.push("firefly-core.exe");
    let core_process = Command::new(core_path).arg(ui_to_core_channel_id).arg(core_to_ui_channel_id).spawn().expect("UI: start core failed");
    let mut window_state = WindowState {
        core_process, ui_to_core_shmem, core_to_ui_shmem,
        d2d_factory, dwrite_factory,
        render_target: None, text_format: None, black_brush: None,
        lines: vec!["".to_string()],
    };
    unsafe {
        let instance = GetModuleHandleA(None)?;
        let class_name = w!("FireflyWindowClass");
        let wc = WNDCLASSW { style: CS_HREDRAW | CS_VREDRAW, hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize), hCursor: LoadCursorW(None, IDC_ARROW)?, lpfnWndProc: Some(window_proc), hInstance: instance.into(), lpszClassName: class_name, ..Default::default() };
        let atom = RegisterClassW(&wc);
        if atom == 0 { panic!("UI: register class failed"); }
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE::default(), class_name, w!("Firefly Editor"), WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, 800, 600, None, None, instance, Some(&mut window_state as *mut _ as _));
        if hwnd.0 == 0 { panic!("UI: create window failed"); }
        let mut message = MSG::default();
        loop {
            if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).into() {
                if message.message == WM_QUIT { break; }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            } else {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if state_ptr == 0 { continue; }
                let state = &mut *(state_ptr as *mut WindowState);
                let shmem_slice = state.core_to_ui_shmem.as_slice();
                let mut len_bytes = [0u8; 4];
                len_bytes.copy_from_slice(&shmem_slice[..4]);
                let msg_len = u32::from_le_bytes(len_bytes) as usize;
                if msg_len > 0 {
                    let msg_data = &shmem_slice[4..4 + msg_len];
                    match serde_json::from_slice(msg_data) {
                        Ok(DrawCommand::RenderLine { line_num, text }) => {
                            if line_num >= state.lines.len() { state.lines.resize(line_num + 1, String::new()); }
                            state.lines[line_num] = text;
                            InvalidateRect(hwnd, None, false);
                        }
                        Err(e) => eprintln!("[UI] 错误: 反序列化绘图指令失败: {}", e),
                    }
                    let shmem_slice_mut = state.core_to_ui_shmem.as_slice_mut();
                    shmem_slice_mut[..4].copy_from_slice(&0u32.to_le_bytes());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CREATE {
        let create_struct = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create_struct.lpCreateParams as isize);
        return LRESULT(0);
    }
    
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 { return DefWindowProcW(hwnd, msg, wparam, lparam); }
    let state = &mut *(ptr as *mut WindowState);

    match msg {
        WM_CHAR => {
            if let Some(char_code) = char::from_u32(wparam.0 as u32) {
                if !char_code.is_control() {
                    let op = Op::InsertChar(char_code);
                    let serialized_op = serde_json::to_vec(&op).unwrap();
                    let len = serialized_op.len() as u32;
                    let shmem_slice = state.ui_to_core_shmem.as_slice_mut();
                    shmem_slice[..4].copy_from_slice(&len.to_le_bytes());
                    shmem_slice[4..4 + serialized_op.len()].copy_from_slice(&serialized_op);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            create_graphics_resources(state, hwnd).expect("UI: 创建绘图资源失败");
            if let (Some(rt_hwnd), Some(brush), Some(format)) = (&state.render_target, &state.black_brush, &state.text_format) {
                // ---- 这是最关键的修正之二：使用 cast::<T>() 进行接口转换 ----
                if let Ok(rt) = rt_hwnd.cast::<ID2D1RenderTarget>() {
                // --------------------------------------------------------
                    unsafe {
                        rt.BeginDraw();
                        rt.Clear(Some(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
                        for (i, line) in state.lines.iter().enumerate() {
                            let text: Vec<u16> = line.encode_utf16().collect();
                            let layout_rect = D2D_RECT_F { left: 10.0, top: 10.0 + (i as f32 * 20.0), right: 800.0, bottom: 600.0 };
                            rt.DrawText( &text, format, &layout_rect, brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL );
                        }
                        if let Err(_) = rt.EndDraw(None, None) {
                            state.render_target = None;
                        }
                    }
                }
            }
            ValidateRect(hwnd, None);
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(rt) = &state.render_target {
                let size = D2D_SIZE_U { width: (lparam.0 & 0xFFFF) as u32, height: ((lparam.0 >> 16) & 0xFFFF) as u32 };
                unsafe { let _ = rt.Resize(&size); }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = state.core_process.kill();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
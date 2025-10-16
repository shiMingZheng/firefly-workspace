use shared_memory::*;
use std::env;
use std::time::Duration;
use firefly_ipc::{DrawCommand, Op};
use ropey::Rope;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        panic!("[Core] 错误：启动时未收到通信通道ID。");
    }
    let ui_to_core_channel_id = &args[1];
    let core_to_ui_channel_id = &args[2];

    let mut ui_to_core_shmem = ShmemConf::new().os_id(ui_to_core_channel_id).open().expect("[Core] 错误：无法连接到 ui_to_core 通道");
    let mut core_to_ui_shmem = ShmemConf::new().os_id(core_to_ui_channel_id).open().expect("[Core] 错误：无法连接到 core_to_ui 通道");

    let mut document = Rope::new();
    let mut cursor_pos = 0;

    println!("[Core] 进程启动，并成功连接到共享内存。开始监听指令...");
    
    loop {
        let shmem_slice = unsafe { ui_to_core_shmem.as_slice_mut() };
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&shmem_slice[..4]);
        let msg_len = u32::from_le_bytes(len_bytes) as usize;

        if msg_len > 0 {
            let msg_data = &shmem_slice[4..4 + msg_len];
            let op: Result<Op, _> = serde_json::from_slice(msg_data);

            if let Ok(op) = op {
                println!("[Core] 收到新指令: {:?}", op);

                match op {
                    Op::InsertChar(c) => {
                        document.insert_char(cursor_pos, c);
                        cursor_pos += 1;
                        
                        let line_idx = document.char_to_line(cursor_pos);
                        let line = document.line(line_idx).to_string();

                        let draw_cmd = DrawCommand::RenderLine { line_num: line_idx, text: line };
                        println!("[Core] 文本已更新，准备发送绘图指令: {:?}", draw_cmd);

                        let serialized_cmd = serde_json::to_vec(&draw_cmd).unwrap();
                        let len = serialized_cmd.len() as u32;
                        let core_shmem_slice = unsafe { core_to_ui_shmem.as_slice_mut() };

                        core_shmem_slice[..4].copy_from_slice(&len.to_le_bytes());
                        core_shmem_slice[4..4 + serialized_cmd.len()].copy_from_slice(&serialized_cmd);
                        println!("[Core] 绘图指令发送成功。");
                    }
                }
            }
            shmem_slice[..4].copy_from_slice(&0u32.to_le_bytes());
            println!("[Core] 指令处理完毕，输入缓冲区已清空。");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
} // <--- 已删除这里多余的 '}'
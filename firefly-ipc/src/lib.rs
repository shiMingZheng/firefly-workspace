// 引入 serde 库，用于序列化和反序列化
// 我们需要能够将这些结构体转换成字节流，以便在共享内存中传输
use serde::{Deserialize, Serialize};

/// IPC 指令 (Operation)
/// 从 UI 进程发送到 Core 进程
#[derive(Serialize, Deserialize, Debug)]
pub enum Op {
    /// 请求插入一个字符
    InsertChar(char),
    // 未来会在这里添加更多指令，如移动光标、删除等
}

/// 绘图指令 (Draw Command)
/// 从 Core 进程发送到 UI 进程
#[derive(Serialize, Deserialize, Debug)]
pub enum DrawCommand {
    /// 请求渲染指定行号的文本
    RenderLine { line_num: usize, text: String },
    // 未来会在这里添加更多指令，如绘制光标、绘制选区等
}
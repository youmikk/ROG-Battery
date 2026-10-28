//! 构建脚本：把 `vendor\mingw-import-libs\` 加进链接器的库搜索路径。
//!
//! 为什么需要它：本机 rustlib 的 `self-contained` 目录里那 43 个 import library
//! **没有 `libgdiplus.a`**（也没有 `libshlwapi.a` / `libdwmapi.a` / `libuxtheme.a`），
//! 而 GDI+ 自绘要用到 `gdiplus.dll`。缺了它，`#[link(name = "gdiplus")]` 会变成
//! 链接期的 `ld: cannot find -lgdiplus`。
//!
//! 那两个库来自 LLVM-MinGW（`tools\vendor-import-libs.ps1` 负责从本机拷出来）。
//! 只拷**这 5 个**，绝不能把整个 LLVM-MinGW lib 目录加进搜索路径 ——
//! 它的 `libmsvcrt.a` / `libmingwex.a` 会盖过 rustlib 的同名库，CRT 起步直接崩。
//!
//! 注意：`gdiplus.dll` 自 Windows XP 起就是系统组件，**不进安装包、不额外分发**。
//! 这里加搜索路径只是为了链接期能找到导入符号。

use std::path::Path;

fn main() {
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/mingw-import-libs");

    // 目录存在才加：没跑过 `tools\vendor-import-libs.ps1` 的机器上，
    // 只要不引用 gdiplus（例如只构建 CLI 子集）也不该因为它而构建失败。
    if vendor.is_dir() {
        println!("cargo:rustc-link-search=native={}", vendor.display());
    } else {
        // 不 panic：让人看到提示，但让真正缺库时的链接错误来说话，
        // 因为错误信息里有 `-lgdiplus`，比这里的 panic 更容易定位。
        println!(
            "cargo:warning=vendor/mingw-import-libs 不存在，GDI+ 相关目标可能链接失败。\
             先运行: powershell -NoProfile -ExecutionPolicy Bypass -File tools/vendor-import-libs.ps1"
        );
    }

    link_app_icon();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/mingw-import-libs");
}

/// 把 `assets\app.o`（编译好的图标资源）链进 exe。
///
/// # 为什么用预编译的 `.o` 而不是在这里调 `windres`
///
/// 资源脚本 `assets\app.rc` 需要 `windres` 编译成 COFF 才能链接。rustlib 的
/// `x86_64-w64-mingw32-gcc.exe` 旁边**没有** windres，本机唯一可用的是
/// LLVM-MinGW 的 `llvm-windres.exe`（在 winget 包目录里）。
///
/// 但**在这里调它行不通**：`Command::new(windres)` 报
/// `Unable to create temp file: Permission denied` —— 构建脚本派生的子进程
/// 拿到的是受限令牌（Low 完整性 + 受限 SID），`%TEMP%` 不可写；
/// 把 `TMP`/`TEMP` 指到项目内目录也没用（令牌限制，不是路径问题）。
///
/// 所以：**`.o` 预先生成好并提交进仓**，构建脚本只负责把路径告诉链接器。
/// 重新生成的方式（在普通 powershell 里跑，不要在 cargo 派生的进程里跑）：
///
/// ```text
/// llvm-windres -O coff -I assets -i assets\app.rc -o assets\app.o
/// ```
///
/// # 为什么 `rustc-link-arg` 而不是 `rustc-link-lib`
///
/// 这是个 `.o` 文件不是库，链接器要的是**绝对路径**。`rustc-link-arg`
/// 把参数原样透传给链接器 —— 注意它只对**当前 crate 的最终产物**生效，
/// 对这个纯 bin crate 正好。
fn link_app_icon() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let obj = root.join("assets/app.o");
    if obj.is_file() {
        println!("cargo:rustc-link-arg={}", obj.display());
    } else {
        // 不 panic：没有图标程序照样能跑（Windows 会给个通用占位图标），
        // 不该因为一个图标挡住整个构建。
        println!(
            "cargo:warning=assets/app.o 不存在，exe 将没有图标资源。\
             生成方式: llvm-windres -O coff -I assets -i assets/app.rc -o assets/app.o"
        );
    }
    println!("cargo:rerun-if-changed=assets/app.o");
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/tray.ico");
}

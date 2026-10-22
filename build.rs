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

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/mingw-import-libs");
}

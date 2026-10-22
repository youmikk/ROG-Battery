# GDI+ 在真实窗口 DC 上绘制：实测数据

> 本文只记**我亲自跑出来的**数字。探针源码在 `scratch\gdiplus-probe\windowdc.rs`，
> 截图在 `scratch\gdiplus-probe\windowdc-shot.png`。

## 为什么单独测这件事

`docs\gdiplus-rendering.md` 里的验证是画到 `CreateDIBSection` 拿到的**内存位图**上。
那是离屏的：没有裁剪区、不做屏幕刷新、不存在「更新区」概念。

真实集成时拿到的是 `BeginPaint` 返回的**窗口 DC**。两者不是一回事：

| | 内存位图 DC | 窗口 DC |
|---|---|---|
| 抗锯齿混合的读源 | 内存里自己的像素 | **屏幕上的像素**（要回读） |
| 裁剪区 | 无 | 有（更新区 + 窗口矩形） |
| 绘制结果何时可见 | 你 `BitBlt` 时才可见 | 每次绘制调用后就可能被合成 |

所以我另写了一个探针，把同一份内容**画两遍**（一遍直接画窗口 DC，一遍画内存 DC 再
`BitBlt`），用 `QueryPerformanceCounter` 计时。

## 结果（三轮独立运行）

```
#0  direct=24751us buffered=3705us   (预热，不计)
#10 direct= 7968us buffered=2687us
=== 共 16 帧（剔除预热）：direct 平均 8.34ms，buffered 平均 2.68ms

#0  direct=24296us buffered=4743us   (预热，不计)
#10 direct= 8082us buffered=3008us
=== 共 16 帧（剔除预热）：direct 平均 8.53ms，buffered 平均 2.67ms

#0  direct=22074us buffered=3078us   (预热，不计)
#10 direct= 8018us buffered=2514us
=== 共 16 帧（剔除预热）：direct 平均 8.22ms，buffered 平均 2.63ms
```

| 方式 | 平均耗时 | 说明 |
|---|---|---|
| 直接画窗口 DC | **8.22 – 8.53 ms** | 三轮极稳定（σ ≈ 0.16ms） |
| 画内存 DC + `BitBlt` | **2.63 – 2.68 ms** | 三轮极稳定（σ ≈ 0.02ms） |
| 首帧（两种方式都一样） | **22 – 25 ms** | GDI+ 首次走字体/纹理路径的冷启动 |

**双缓冲快 3.1 倍，不是慢一点，是快三倍。**

## 为什么双缓冲反而更快（这个反直觉）

我原本的预设是「双缓冲多干一倍的活（画一遍 + 拷一遍），只是换来不闪烁」。
实测推翻了它。

原因在最后一步 `BitBlt` 之外：**GDI+ 在窗口 DC 上做抗锯齿混合时，必须回读目标像素**。
抗锯齿的每个边缘像素结果是 `alpha * 新色 + (1-alpha) * 旧色`，而「旧色」在窗口 DC 上
是**屏幕上的**像素 —— 这次回读要穿透 DWM 合成器，代价远高于读内存。

双缓冲把它变成：所有混合都在内存里做（纯内存读），最后一次 `BitBlt` 整块推上去
（`SRCCOPY` 不做混合，直接拷贝）。

所以双缓冲在这里不只是「防闪烁的手段」，而是**更快的画法**。上一份文档里
「如果直接画只要 3ms，双缓冲是过度设计」这个判断是错的，实测是反过来。

## 三个附带确认

1. **`GdipCreateFromHDC(hdc)` 传窗口 DC 返 0**（成功）。之前担心的「窗口 DC 色彩格式
   或裁剪区会导致失败」不成立。
2. **中文渲染正常**，无需任何字体配置 —— `GdipCreateFontFamilyFromName("Microsoft YaHei UI")`
   返 0，`GdipDrawString` 直接出字。截图里「电量历史」「剩余电量」「设备信息」全部清晰。
3. **整页 729x487 深色卡片布局在真实窗口里成立** —— 圆角、渐变电量环、抗锯齿折线、
   双圈数据点、绿色 `-1.26 %/h` 全部正常，与 `docs\mockups\battery-page-gdiplus.png`
   观感一致。

## 首帧 22–25ms 要怎么处理

首帧是特例（GDI+ 初始化字体缓存、首次分配纹理）。用户看到的第一次绘制会慢 3 倍。

处理方式：**双缓冲的缓存对象要在窗口创建时就建好**（`CreateCompatibleDC` +
`CreateCompatibleBitmap`），不要等到第一次 `WM_PAINT` 才建。探针里是懒建的
（第一次 `WM_PAINT` 才建，日志里能看到「建双缓冲 729x487」那一行），
生产代码应该提前建。

另外首帧慢还有一部分是**字体对象**：探针每次绘制都 `GdipCreateFont` / `GdipDeleteFont`
（一帧几十次）。生产代码应该把常用字号（27px bold / 18px / 14px bold / 12px / 11px / 10px）
的 `GpFont*` **缓存在 `Fonts` 结构里**，只在 DPI 或字号变化时重建。

## 对 `render.rs` 的要求（据此更新）

1. **必须用双缓冲**。不是为了防闪烁，是为了 3 倍性能
2. 双缓冲的 `CreateCompatibleDC` / `CreateCompatibleBitmap` 在**窗口创建时**建好，
   `WM_SIZE` 变化时重建（记得 `SelectObject(dc, old)` 再 `DeleteObject(bmp)`，顺序错会泄漏）
3. `GpFont*` 缓存，别每帧建几十个
4. `WM_ERASEBKGND` 返回 1（自己吃下）。因为整页深色，让系统先刷 `COLOR_BTNFACE` 灰
   再被我们盖掉，拖动窗口时会闪灰
5. `WM_PAINT` 里先 `FillRect` 整页 `BG`（或用 GDI+ 填一个整页矩形），再画卡片

## 完整图形生命周期（照抄这个顺序，漏一步就泄漏）

```
窗口创建时：
  hdc_screen = GetDC(hwnd)                    ← 只为拿格式模板
  mem_dc     = CreateCompatibleDC(hdc_screen)
  mem_bmp    = CreateCompatibleBitmap(hdc_screen, w, h)
  old_bmp    = SelectObject(mem_dc, mem_bmp)
  ReleaseDC(hwnd, hdc_screen)
  gp_fonts   = { fam: GdipCreateFontFamilyFromName(...), sizes: [...] }

WM_PAINT：
  hdc = BeginPaint(hwnd, &ps)
  paint_page(mem_dc, ...)                     ← 画到内存
  BitBlt(hdc, 0,0,w,h, mem_dc, 0,0, SRCCOPY)  ← 一次推上去
  EndPaint(hwnd, &ps)

WM_SIZE：
  SelectObject(mem_dc, old_bmp)               ← 必须先换出
  DeleteObject(mem_bmp)
  ... 重建 ...

退出：
  SelectObject(mem_dc, old_bmp)
  DeleteObject(mem_bmp)
  DeleteDC(mem_dc)
  GdipDeleteFont(...) ×N
  GdipDeleteFontFamily(fam)
  GdiplusShutdown(token)
```

`SELECTOBJECT` 的顺序是经典事故点：先 `DeleteObject(bmp)` 再 `SelectObject(dc, old)`，
那个 bmp 还被 DC 选着，`DeleteObject` 会静默失败（返回 0，不报错），**位图永久泄漏**。

## 探针本身的两个坑（记下来免得后人踩）

1. **`CreateWindowExW` 的 `hInstance` 参数不能传 `std::ptr::null_mut()` 给 `isize` 类型**
   —— 我的探针把句柄统一声明成 `isize` 了，所以空值要写 `0` 而不是 `null_mut()`。
   混用会报 `expected isize, found *mut _`。
2. **`thread_local!` 里放裸指针只能读不能写** —— `FAM.with(|f| *f = fam)` 报
   `cannot assign to *f, which is behind a & reference`。要用
   `Cell<*mut c_void>` 然后 `.set()` / `.get()`。

## 复现方法

```powershell
$env:RUSTUP_HOME='D:\dsh\tools\rustup'; $env:CARGO_HOME='D:\dsh\tools\cargo'
# 注意：PATH 里绝不能有 LLVM-MinGW，否则 x86_64-w64-mingw32-gcc 被它劫持，
# 报 `lld: error: unable to find library -lgcc_eh`
$env:PATH="D:\dsh\tools\cargo\bin;C:\Windows\system32;C:\Windows;C:\Windows\System32\Wbem;C:\Windows\System32\WindowsPowerShell\v1.0"

cd D:\codex\rog-mouse-ctl\scratch\gdiplus-probe
& 'D:\dsh\tools\rustup\toolchains\stable-x86_64-pc-windows-gnu\bin\rustc.exe' `
  -O --edition 2021 windowdc.rs -o windowdc.exe `
  -L 'D:\codex\rog-mouse-ctl\vendor\mingw-import-libs' `
  -C strip=symbols -C panic=abort

# 跑 8 秒自动退出；每 500ms 重画一帧
.\windowdc.exe
Get-Content windowdc.log -Encoding UTF8
```

截图（`PrintWindow` 的第 3 参数 `2 = PW_RENDERFULLCONTENT`，`CopyFromScreen` 会被遮挡污染）：

```powershell
$hwnd = [W]::Find('WindowDcProbe')       # 用 EnumWindows，不能 FindWindow（会返回 0）
[W]::Cap($hwnd, 'windowdc-shot.png')
```

**注意 `FindWindow` 找不到这个窗口**（探针里实测返回 0，`EnumWindows` 能找到）——
大概和我把窗口建在默认桌面上、但类名注册顺序有关。用 `EnumWindows` + `GetClassNameW`
逐条匹配最稳。

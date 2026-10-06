# Format Forge · 批量格式转换器

本地批量格式转换桌面应用。拖入文件，选目标格式，调参数，一键转换。全部处理在本机完成，文件不出本机。

## 运行

```bash
npm install
npm run dev        # tauri dev，同时起 Vite 与 Rust 后端
```

前端单独跑（无后端）用 `npm run dev:vite`，端口固定 5200。

## 打包

```bash
npm run build      # 产出 NSIS 安装包到 src-tauri/target/release/bundle/
```

## 能力范围

**图片** —— PNG / JPEG / WebP / GIF / BMP / TIFF / ICO 之间互转；尺寸缩放、EXIF 剥离。
**质量滑杆只对 JPEG 出现** —— `image` crate 的 WebP 编码器只做无损，`save_with_format`
会把 quality 整个丢掉（实测 q10 与 q95 产出字节完全相同）。与其放一个拖了没反应的滑杆，
不如让它别出现。

不做 SVG / HEIC / AVIF：前者要 resvg 栅格化，中者要系统 HEIF 扩展（本机 `.heic`
的默认处理程序是第三方看图工具，不能假定 WIC 可用），后者要开 `image` 的 `avif`
feature（ravif 一整套依赖）。**注册表里只登记真正实现了的格式** —— 可选却一定失败的
条目比没有还糟，用户选了才在转换阶段撞到「暂不支持」。

**表格与数据** —— XLSX ↔ CSV / TSV / JSON。多工作表的 xlsx 转 CSV 时，`sheet_mode`
决定产物形态：**每个工作表一个文件**（默认，`台账-华东.csv`）/ **合并为一个文件** /
**只取第一个工作表**。合并按**列名**并集而不是按位置首尾相接——各表列数不同时位置对齐
会让数据整体错位；某张表有别的表没有的列时补空。
编码可选 UTF-8 / UTF-8 BOM / GBK，读取时自动识别 BOM 并回退 GBK。

**文档文本** —— DOCX → TXT / Markdown / HTML。纯 Rust 解析，不需要装 Office：
标题级别、粗体、超链接、列表、表格、分页符都会保留；版面（浮动文本框、页眉页脚）不保留。

**PDF 操作** —— 合并（多个 PDF 拼成一个）、拆分（按页码范围或每 N 页）、
页面整理（旋转 / 删页 / 重排）、提取文本。走 lopdf 纯 Rust 路线，
页面对象原样搬运，**内容流不解码重编码**，所以这些操作是真正无损的。

**Office → PDF** —— 见下节。这是唯一需要装 Office 的能力。DOCX / XLSX / PPTX 都可转，
但 PPTX 的目标只有 PDF（幻灯片转图要走 `Presentation.Export`，与现在的 `SaveAs` 是
两条不同的 COM 路径，还没写）。

不做视频、不做音频。不支持同格式转换（PNG→PNG）。

## Office → PDF 的保真度

这是本应用的核心能力，按保真度从高到低探测可用引擎：

| 引擎 | 保真度 | 说明 |
|---|---|---|
| Microsoft Office COM | 与 Office 中所见完全一致 | 需要装 Office |
| WPS Office COM | 近乎一致 | 中文 Windows 上常见 |
| LibreOffice headless | 会重新排版 | 免费方案，复杂文档分页可能不同 |

启动时探测一次，结果显示在右上角芯片里，点击可看详情与安装引导。
**绝不把 LibreOffice 说成和 Office 一样** —— 保真度以本机真实可用引擎为准如实标注。

探测归探测，**目前只有 Office COM 一条路真的实现了**。「转换引擎」选单里选 WPS 或
LibreOffice 会被如实拒绝（`unsupported`），而不是偷偷用 Office 转完假装是别家结果。
`pdf_engine` 的 `auto` / `office` 两个值都走 Office。

## 有意没做的

评估过，结论是不做，理由写在这里免得后来者重复踩：

**PDF 水印** —— 需要往内容流里插绘图指令，还要自己算字体、变换矩阵与图形状态栈。
做砸了会毁掉正文（PDF 的内容流是一段栈式程序，插入点错一个字节整页就废）。
收益是一个锦上添花的功能，风险是损坏用户文件。宁可不做。

**PDF 压缩** —— 真正的压缩要么重压内嵌图片（需要完整的图像解码重编码管线），
要么只是重新序列化对象（收益通常只有几个百分点，远不如用户预期）。
两者性价比都不对，做了反而给人「压缩无效」的印象。

**PDF → 图片 / 图片 → PDF** —— 需要 pdfium（渲染）与图像合成。
pdfium 是 C++ 库，要分发一个 7MB 的 DLL 并处理动态绑定，是独立的一块工程量。
所以 `pdf` 的 `targets` 里**没有** `png` / `jpeg`，`render_dpi` 参数也一并撤了——
留着这两个的话，用户选了 PDF→PNG 会得到「尚未接入」，而那本该是个不该出现的选项。

**PPTX → PNG / JPEG** —— 幻灯片转图要走 `Presentation.Export(路径, "PNG", 宽, 高)`，
与现在 `SaveAs($dst, ppSaveAsPDF)` 是两条不同的 COM 路径。没写，所以 `pptx` 的
`targets` 里只有 `pdf`，那对 96/150/300 DPI 的选项也跟着撤了。

## 结构

```
src/                      前端（React + Vite，纯 JSX）
  components/             按组件拆分的 .jsx，无 barrel 文件
  hooks/                  use*.js，具名导出
  lib/
    api.js                invoke 包装
    actions.js            动作型目标（@split / @merge）的表现层元数据
    targets.js            目标解析：参数归属、计数、展示名
    format-meta.js        格式徽标的颜色与短标签
    format.js             字节/时长/百分比的格式化
  styles/global.css       单一全局样式表（设计令牌 + 全部组件样式）
src-tauri/
  src/engine/probe.rs     引擎探测（只读注册表与文件系统）
  src/engine/office.rs    COM worker 的 Rust 侧管理（启动/协议/超时/清理）
  src/convert.rs          单文件转换的分派入口（图片、Office→PDF）
  src/table.rs            XLSX ↔ CSV / TSV / JSON
  src/docx.rs             DOCX → TXT / MD / HTML
  src/pdf.rs              PDF 合并 / 拆分 / 整理 / 取文本
  src/job.rs              批量作业模型、双泳道调度、事件节流
  src/scan.rs             文件夹递归、魔数嗅探、输出命名与去重、目标解析
  src/formats.rs          格式嗅探（含 OOXML 压缩包识别）
  src/registry.rs         格式注册表（能力与参数 schema 的唯一来源）
  src/paths.rs            长路径（\\?\）包装与显示用路径
  src/commands.rs         Tauri 命令层
  src/testkit.rs          端到端测试的管线入口（非产品代码）
  scripts/office-worker.ps1  COM 常驻 worker（PowerShell）
  tests/pipeline.rs       端到端管线测试
  tests/registry_shape.rs  注册表序列化的跨语言契约测试
mock/                     浏览器 UI 验证台（假数据，不参与打包）
  harness.jsx             把 Tauri 的 invoke 换成假实现
  tauri-stub.js           getCurrentWebview 的空实现
  formats.json            注册表快照，供假数据用
scripts/
  check-props.mjs         组件 props 一致性检查（堵「传漏了」导致的黑屏）
  make-icons.mjs          生成应用图标（手写 PNG/ICO 编码，零依赖）
  make-fixtures.mjs       生成 Office 保真度测试样本
  test-office-worker.mjs  命令行测试 Office 转换
  diag-excel.ps1          Excel COM 诊断
```

## 格式注册表是唯一来源

`src-tauri/src/registry.rs` 定义每个格式能转成什么、有哪些参数。
前端启动时通过 `list_formats` 拉一次，**不维护第二份能力表** ——
目标选单、参数面板都从这里推导，加一个格式只改 Rust 一处。

目标是**动作**时（`@split`、`@merge`、`@organize`），它是注册表 `targets`
里带 `@` 前缀的条目。参数不另起一张表，而是挂在动作**产出的格式**上
（见 `src/lib/actions.js` 的 `owner` 字段），扫描阶段再把动作翻译成
「具体输出格式 + 注入的参数」（见 `scan.rs` 的 `resolve_target`）。

## 测试

```bash
cd src-tauri && cargo test
```

前端侧还有一条不需要测试框架的检查：

```bash
npm run check:props     # 组件解构的 props 与调用点传的是否对得上
```

界面本身可以在**浏览器**里跑起来看（真机上 UI 依赖 Tauri 的 invoke，
浏览器里起不来，所以验证台把 invoke 换成假数据）：

```bash
npx vite --config vite.mock.config.js    # 然后开 http://localhost:5210
```

`?scene=crash` 会故意抛一个异常，用来看错误边界长什么样；`?scene=once`
用来验证「重试」确实重挂了组件树。这个验证台不参与打包，也不需要它参与——
它存在的理由是：**有些 UI 缺陷在类型、lint、构建里都看不出来，只能真的渲染一遍**。

覆盖三层：

* **单元测试**（23 个）—— 渲染、嗅探、编码回退、页码范围解析、旋转归一化这些纯函数。
* **端到端管线测试**（`tests/pipeline.rs`，16 个）—— 在临时目录里造真实文件，
  跑「扫描 → 规划 → 转换 → 校验产物」全流程，断言产出的字节或结构。
* **跨语言契约测试**（`tests/registry_shape.rs`，5 个）—— 守住注册表序列化出来的
  JSON 形状（`kind` 的 PascalCase 取值、Select 的 choices、Slider 的 min/max、
  targets 不含自身），以及**能力表与后端实现的一致性**。

后三条尤其值得说明，它们堵的是同一类漏洞——**注册表说了、代码没做**：

* `every_target_is_implemented` —— 每个目标要么是个真格式，要么被别的格式声明为目标。
* `every_option_key_is_read_by_the_backend` —— 每个参数都有后端代码读它。
  这条是补上的：`slide_export_dpi` 与 `render_dpi` 曾在表里挂了很久，
  UI 上两个滑杆真实可见、拖动也真有状态，只是从来没有任何一行 Rust 读过。
* `registry_has_no_formats_we_cannot_actually_handle` —— AVIF 曾是反例：
  `can_encode: true`、列在每个图片格式的 targets 里，但 `Cargo.toml` 没开
  `image` 的 `avif` feature，选中它必然失败。

端到端测试**不碰 COM**（那需要装 Office），Office 那条路由
`scripts/test-office-worker.mjs` 单独覆盖。

## 几个值得记住的坑

**Word 与 Excel 的可选 COM 参数必须传 `[Type]::Missing`。**
传 `''` 给 Word 会得到「这是一个无效文件名」，传 `$null` 给 Excel 会得到「不能取得类 Workbooks 的 Open 属性」。
两个报错都极具误导性——一个像路径问题，一个像权限问题，实际都不是。

**PowerShell 脚本里的内部变量不能叫 `$App`。**
`New-Object -ComObject $App` 触发了 PowerShell 的自动变量绑定，会把 `$App` 覆盖成 CLSID 字符串，随后参数校验失败。
现在叫 `$script:Com`。

**`office-worker.ps1` 必须保持纯 ASCII。**
Windows PowerShell 5.1 按系统 ANSI 码页读 `.ps1`，中文源码会被破坏。错误分类与面向用户的文案都放在 Rust 侧。

**Word 实例化会连带启动一个有窗口的兄弟进程。**
只调 `Quit()` 不够，收尾时要按启动时间杀掉整个进程族。

**`serde` 的 `rename_all` 不改字段名。**
枚举变体要用 `rename_all = "camelCase"`，字段名要额外加 `rename_all_fields = "camelCase"`。少了后者前端会收到 `bytes_out` 而它期待 `bytesOut`。

**Tauri 的 `dragDropEnabled: true` 会吞掉 HTML5 拖放事件。**
用 `getCurrentWebview().onDragDropEvent()` 取路径，它给的是真实文件系统路径，文件夹也能拿到。

**`csv` crate 默认吃掉第一行。**
`ReaderBuilder` 的 `has_headers` 默认为 true，`records()` 会跳过表头。
如果自己也去取一次 header，就会静默丢一行数据。要先 `has_headers(false)` 再自己管。

**`delete_pages` 之后页码会重排。**
所以「删页 + 重排」不能拿源页码去索引输出位置，必须先做一次
「源页码 → 删除后页码」的翻译。`pdf.rs` 的 `apply_plan` 里注释写清了这一点。

**`registry.rs` 里的 `Select` 参数值是字符串。**
`pdf_rotate` 写成 Select 后，`spec.num("pdf_rotate")` 永远拿到默认值 0 ——
得用 `spec.text(...).parse()`。这类「参数类型与读取方式不匹配」的 bug 不会报错，只会静默失效。

**DLL 风格的 `lopdf` 保存返回 `std::io::Error` 而不是 `lopdf::Error`。**
`Document::save` 的 Err 类型是 std 的，别拿去喂给 `classify_lopdf`。

**`image` crate 的 WebP 编码器只做无损。**
所以 `save_with_format` 会把 `quality` 整个丢掉——实测 q10 与 q95 产出**字节完全相同**。
要真有损得走 `image-webp` 的 lossy 路径。当前的做法是让质量滑杆对 WebP 不出现。
顺带一提 `Decoder::new_with_quality` 只对 JPEG 有效，其余格式静默忽略。

**组件解构了某个 prop 而调用点没传，会整窗黑屏。**
`FileQueue` 曾解构了 `collapsing` 却没人传，`collapsing.has(...)` 抛 TypeError；
项目里没有 ErrorBoundary，React 一吐异常就把整棵树卸掉——**表现是「拖进文件后
整个窗口变黑」，而 oxlint 和构建都毫无反应**。这类「传漏了」的崩溃靠
`npm run check:props` 兜住（它把每个组件解构的 props 与调用点传的 props 对一遍），
它就是为了这个 bug 写的。往组件加 prop 时记得在调用点补上。

**注册表里「说了但没做」的条目比没有还糟。**
AVIF 曾经可选却必然失败；`slide_export_dpi` / `render_dpi` 两个滑杆 UI 上看得见、
拖动也真有状态，却没有任何后端代码读它们。`tests/registry_shape.rs` 现在有三条
断言守着这类偏差——**加格式或加参数时先看清楚后端有没有对应分支**。

## 开发环境

MSVC 工具链与 Windows SDK 都已具备，**直接 `cargo build` / `cargo test` 即可**，不需要先初始化 VS 环境。

唯一需要注意的是 **Windows SDK 装在 `D:\Windows Kits\10`（非默认路径）**。
Rust 的 `cc`/`link` 通过注册表 `HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots`
定位 SDK，实测能自动找到。只有在某些终端环境里这条继承链断了、
`cargo build` 报 `LNK1181: 无法打开输入文件"kernel32.lib"` 时，才需要退回手动初始化：

```bash
cmd /c ""C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo build"
```

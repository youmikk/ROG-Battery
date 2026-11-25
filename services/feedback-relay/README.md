# X-Battery feedback relay

X-Battery 的自有反馈中继：桌面客户端通过 HTTPS 提交设备反馈，服务端在固定仓库 [`youmikk/X-Battery`](https://github.com/youmikk/X-Battery) 创建议题，用户可用本机保存的回复密钥查看维护者回复和补充信息。客户端不需要 GitHub 登录，也不会持有 GitHub Token。

服务独立实现，运行时无 npm 依赖。使用 Cloudflare Worker、一个 SQLite Durable Object 和一个私有 R2 桶。无需 GitHub 附件上传 API，不调用 MyFinder 或其他项目的反馈服务器。

## 文件和本地测试

- `src/index.mjs`：HTTP 路由、限流、持久化请求记录和截图存储。
- `src/github.mjs`：仅访问固定仓库的 GitHub API；创建议题、恢复提交结果、查看维护者回复。
- `src/security.mjs` / `src/validation.mjs`：认证、输入限制、GitHub URL 校验、文件格式检查。
- `test/relay.test.mjs`：原生 Node 测试；GitHub、R2、Durable Object 存储全部由内存模拟器替代。

在本目录使用 Node 22 或更新版本运行：

```powershell
node --test test/relay.test.mjs
```

测试不会读取真实凭据、联网创建议题或部署服务。测试使用模拟时间、HTTP 响应和图片协议样本，不能替代 Cloudflare 上的部署验收。

## 部署配置

需准备自己的 Cloudflare 账户、Worker 域名、Durable Objects 和 R2。R2 桶保持私有，不开启公开 `r2.dev` 地址。下面的操作由维护者在确认配置后执行；源码交付本身不会执行部署。

1. 安装 Wrangler 4，并登录自己的 Cloudflare 账户。Wrangler 是部署工具，不是运行时依赖。按部署环境的要求配置 npm 缓存与 Wrangler 日志位置。
2. 在本目录创建私有桶：

   ```powershell
   wrangler r2 bucket create r-battery-feedback-attachments
   ```

3. 修改 `wrangler.jsonc`：

   | 配置 | 含义 |
   | --- | --- |
   | `vars.PUBLIC_BASE_URL` | 客户端实际访问的 HTTPS 源，例如 `https://r-battery-feedback.<自己的子域>.workers.dev`。不允许路径、用户名、查询参数或非标准端口。末尾 `/` 可省略。 |
   | `vars.GITHUB_AUTHOR_LOGIN` | GitHub Token 所属账号的准确 login；GitHub App 使用实际发帖的 `name[bot]`。用于核对超时后的议题/评论确实由服务账号创建。 |
   | `r2_buckets[0].bucket_name` | 第 2 步创建的桶名。绑定名必须为 `ATTACHMENTS`。 |
   | `durable_objects.bindings` / `migrations` | 保持所附 `FeedbackCoordinator`、SQLite 迁移和 `FEEDBACK` 绑定。首次部署由 Wrangler 建立持久化对象。 |

4. 为 `youmikk/X-Battery` 创建 GitHub fine-grained Token，只授权这个仓库的 **Issues: Read and write** 和必要的 Metadata 读取。组织仓库如有审批要求，先完成审批。建议用专用服务账号便于区分用户转发与维护者直接回复。不要把 Token 放进 Rust、HTML、JSON 配置或 Git 仓库。
5. 使用交互式 secret 输入配置凭据：

   ```powershell
   wrangler secret put GITHUB_TOKEN
   wrangler secret put FEEDBACK_SECRET
   ```

   `FEEDBACK_SECRET` 必须是独立随机生成的 32 字节，以 64 位十六进制保存。它用于回复密钥和 IP 的 HMAC；不是 GitHub Token，也不是任何用户的 `replyKey`。不要使用文档或测试值。通过密码管理器等安全方式保存备份，不要把值写进命令历史。

6. 在本目录执行部署：

   ```powershell
   wrangler deploy
   ```

7. 检查 `GET /health` 返回 `{"status":"ok","schema":1}`，再把同一个 HTTPS 源配置给桌面客户端。该接口只确认配置完整，不会调用 GitHub 或验证 Token 权限。真实发帖验收应使用清楚标注的维护者测试反馈，并在确认后自行关闭。

使用自有域名时，先把域名绑定到此 Worker，再设置 `PUBLIC_BASE_URL`。截图永久引用这个源，部署后应保持域名稳定。

GitHub Token 可更换，但保持发帖账号与 `GITHUB_AUTHOR_LOGIN` 一致，才能恢复该账号仍未确认的旧提交。**不要直接轮换 `FEEDBACK_SECRET`、更改 Durable Object 名称 `coordinator-v1` 或清空持久化记录**：这些变动会破坏旧反馈的回复认证或幂等记录。需要轮换时先设计旧密钥兼容和数据迁移。

`observability.enabled` 默认关闭，服务没有正文、凭据或截图日志。Cloudflare 账户自身的访问日志/分析配置由维护者管理；不要额外开启请求体或 Authorization 日志。`.dev.vars`、`.wrangler` 和本地环境文件已加入本目录 `.gitignore`。

## 客户端协议

所有 JSON 请求设置 `Content-Type: application/json`，所有字符串使用 UTF-8。未知字段会返回 `400`。服务不开放浏览器跨域 CORS；原生 Windows 客户端使用 HTTPS 即可。

### 创建反馈：`POST /feedback`

```json
{
  "schema": 1,
  "requestId": "45a526bf-b747-4d42-81e8-d1036d01f983",
  "replyKey": "客户端为本次反馈生成的64位随机十六进制",
  "appVersion": "1.4.7",
  "category": "battery",
  "device": {
    "name": "我的无线鼠标",
    "family": "mouse",
    "connection": "2.4GHz",
    "brand": "设备品牌",
    "model": "设备型号"
  },
  "description": "鼠标唤醒后电量仍显示未知，手动刷新后恢复。",
  "diagnostics": "可选，客户端准备并供用户检查的诊断文本",
  "images": []
}
```

上例中的 `replyKey` 是说明文字；实际请求必须是客户端使用安全随机数生成器生成的 32 字节随机值的 64 位十六进制。每份新反馈生成新的 UUID 和密钥，在发出请求前持久化。Windows 客户端用 DPAPI 保存回复密钥，不把它写入日志或诊断。

| 字段 | 约束 |
| --- | --- |
| `schema` | 固定数字 `1` |
| `requestId` | UUID，服务归一为小写 |
| `replyKey` | 64 位十六进制；客户端负责随机性，服务端仅保存 HMAC 摘要 |
| `category` | `device` 设备适配、`battery` 电量显示、`dpi` DPI 设置、`other` 其他反馈 |
| `appVersion` | 可选，1–40 字符，只允许数字、英文字母、`.`、`+`、`_`、`-`；当前客户端应总带 |
| `device.name` | 1–120 字符 |
| `device.family` | `mouse`、`keyboard`、`headset`、`gamepad`、`other` |
| `device.connection` | 1–80 字符；例如 `2.4GHz`、`USB`、`Bluetooth`、`unknown` |
| `device.brand` / `device.model` | 可选，分别 1–80 / 1–120 字符；未知时省略 |
| `description` | 去除首尾空白后 5–2000 字符，允许换行 |
| `diagnostics` | 可选，UTF-8 不超过 20 KiB |
| `images` | 可选，最多 3 张；字段见下文 |

设备示例：

```json
{"name":"无线键盘","family":"keyboard","connection":"2.4GHz"}
{"name":"游戏耳机","family":"headset","connection":"USB","brand":"FIFINE","model":"HT6"}
{"name":"无线手柄","family":"gamepad","connection":"Bluetooth"}
{"name":"未知设备","family":"other","connection":"unknown"}
```

成功创建返回 `201`；相同请求编号、内容和密钥再次提交返回 `200`，响应相同：

```json
{"number":123,"url":"https://github.com/youmikk/X-Battery/issues/123","replyAccess":true}
```

### 查看反馈：`GET /feedback/123`

必须带 `Authorization: Bearer <replyKey>`。成功返回：

```json
{
  "issue": {
    "number": 123,
    "url": "https://github.com/youmikk/X-Battery/issues/123",
    "title": "[电量显示] 我的无线鼠标",
    "state": "open",
    "body": "议题正文",
    "createdAt": "2026-11-23T00:00:00Z",
    "updatedAt": "2026-11-23T01:00:00Z"
  },
  "comments": [
    {
      "id": 123456,
      "url": "https://github.com/youmikk/X-Battery/issues/123#issuecomment-123456",
      "body": "已加入测试版，请更新后确认。",
      "author": "youmikk",
      "createdAt": "2026-11-23T01:00:00Z",
      "isMaintainer": true
    }
  ],
  "commentsTruncated": false
}
```

服务读取最近最多 300 条 GitHub 评论，只返回 `OWNER`、`MEMBER`、`COLLABORATOR` 的评论，排除中继发布的“反馈者补充”。维护者身份以 GitHub 返回的关联关系为准。总响应最多 500 KiB；历史过多或响应过大时从较早评论起省略，并设置 `commentsTruncated=true`。客户端可用议题链接查看完整公开历史。较小的结果缓存 30 秒。

### 补充回复：`POST /feedback/123/replies`

同样带 `Authorization: Bearer <replyKey>`：

```json
{"schema":1,"requestId":"c94ea958-d7e7-4d62-b4a0-0cd896c99eb7","body":"更新后已恢复正常。"}
```

`body` 为 1–2000 字符。每条新回复有独立 UUID，重试复用原 UUID 和正文。成功 `201`、幂等重放 `200`：

```json
{"id":123457,"url":"https://github.com/youmikk/X-Battery/issues/123#issuecomment-123457","createdAt":"2026-11-23T02:00:00Z"}
```

凭证只出现在客户端请求体或 Authorization 头，不出现在公开议题/评论、图片 URL或服务日志。正文中若误带当前凭证也会被替换为 `[redacted]`；这不是通用隐私清洗器，客户端仍需控制诊断内容。截图里的像素和图片元数据不会自动去除隐私。

### 超时、去重和错误

创建反馈或回复的结果不能确认时，返回 `202`：

```json
{"status":"pending","retryAfter":5}
```

同时发送 `Retry-After` 头。`retryAfter` 初始 5 秒，后续可能随核对退避增加，最大一小时。客户端保留原请求完整内容、编号和凭证，稍后重试；不要为同一未确认请求生成新编号或自动改走手动发帖。

Durable Object 在 GitHub POST 前先保存请求记录。传输中断后，根据服务生成的公开去重标记查询 GitHub 议题/评论，并核对作者属于 `GITHUB_AUTHOR_LOGIN`。这些标记是请求 HMAC，不是回复凭证，不能用来发回复。服务不会在结果未知时再次 POST。

恢复每批最多查询 5 页，每页 100 条。议题按创建时间从新到旧向后翻页；回复从本轮开始时的末页向前扫描，因此新追加评论不会让续扫重新跳到末尾。回复开始新一轮时另查一次议题取得评论数；后续批次不重复该查询。一次 alarm 最多核对 3 条待恢复记录，即最多 15 次列表查询，另加最多 3 次评论数查询。

每核对完一页且未命中，就在原请求记录保存 `recoveryPage`（下一页）；回复首批也先保存起始页。客户端重试与 alarm 共用该游标，重启后继续扫描。页面读取、格式校验或游标保存失败时保留最后确认的位置；找到回执但完成状态未写入成功时仍重查命中页。完成记录不再保存游标，旧记录缺少游标时从首批开始，无需迁移对象或请求编号。

还有未扫页面时 5 秒后继续，不按页数增加退避；整轮未找到或遇到错误时按原策略退避。到列表末尾（议题也会在早于提交时间一分钟处结束）后将游标重置，下轮重新核对，以处理延迟可见或分页因增删发生移动的情况。跨请求分页不是 GitHub 列表快照；未找到仍返回 `pending`，不会据此补发 POST。

GitHub 没有面向创建议题的事务性幂等键：如果程序恰好在记录 `sending` 后、真正发出 POST 前中断，或 GitHub 已接受但结果长期查不到，请求会保持 `pending`，需要维护者核查。这里优先保证同一请求不会重复发帖。后台 alarm 和客户端重试都会继续核对；不会假称已成功。

相同编号但内容或凭证改变返回 `409 request_conflict`。收到明确 GitHub 拒绝的请求返回 `502 github_rejected`，原请求不会自动重发。维护者修复配置后，用户可在确认旧请求未成功的情况下重新提交。

其他错误格式：

```json
{"error":{"code":"validation","message":"description has an invalid length.","retryable":false}}
```

| 状态 | 常见 code | 客户端处理 |
| --- | --- | --- |
| `400` | `validation` / `invalid_json` / `rendered_too_large` | 修正字段或缩短诊断 |
| `401` / `403` | `unauthorized` / `forbidden` | 检查本机保存的回复密钥；不显示为提交成功 |
| `404` | `not_found` | 议题已不可用或地址不正确 |
| `408` | `request_timeout` | 上传超过 30 秒，保留原请求并稍后重试 |
| `409` | `request_conflict` | 保留原内容重试，或明确创建一份新反馈 |
| `413` / `415` | `request_too_large` / `content_type` | 缩小文件，使用 UTF-8 JSON |
| `429` | `rate_limited` | 按 `Retry-After` 等待；同一请求编号继续使用 |
| `502` | `github_rejected` | 由维护者检查 Token 权限、仓库或 GitHub 限制 |
| `503` | `not_configured` | 显示服务尚未配置，保留输入并提供浏览器手动提交 |
| `503` | `service_busy` | 并发数或正文总预算已满；按 `Retry-After` 等待，继续使用原请求编号 |
| `503` | `service_unavailable` / `github_unavailable` | 保留原请求并稍后重试 |

所有服务返回的 GitHub URL 都必须满足 HTTPS、`github.com`、固定仓库路径和正确议题/评论编号。客户端也应再次校验，不允许凭返回字符串打开其他仓库或主机。

## 截图承载和访问

单张对象格式：

```json
{"name":"截图.png","mimeType":"image/png","dataBase64":"标准base64内容，不带data:前缀"}
```

仅接受 PNG、JPEG、WebP。每张解码后最多 5 MiB，最多三张，总 JSON 请求最多 22 MiB，读取上传正文限时 30 秒。服务检查 MIME 与二进制签名、帧尺寸、最大边长 16384、最大 6000 万像素；不允许 SVG、HTML 或任意附件。不压缩请求体。文件名最多 160 字符，禁止路径分隔符；存储键完全由服务端随机生成，文件名不参与 R2 路径。

R2 保存原始图片，Worker 在 `/attachments/<48位随机hex>` 提供公开只读图片地址。议题中的 Markdown 图片引用这个地址，所以无需公开桶或 GitHub 附件上传权限。虽然地址难猜，**任何获得议题或图片链接的人都能查看图片**。地址和图片不带回复密钥。图片响应设置固定 MIME、`nosniff`、受限 CSP 和无 referrer；浏览器/ GitHub 可能缓存图片。

普通图片格式检查不是完整解码、病毒扫描或图片隐私清理。PNG/JPEG/WebP 的元数据不会重编码去除。客户端应在提交前让用户确认截图不含私密信息。

已提交和结果不明的图片保留，以免议题链接损坏。失败上传、明确被拒的提交在 24 小时后由 alarm 删除已有截图；小型请求摘要仍保留，以防重试重复发帖。公开图片需长期保存时应设置自己的 R2 成本预算；**不要对已发布截图配置统一短期自动删除规则**。用户请求删除截图时，维护者删除对应 R2 对象并编辑议题；外部缓存不能保证即时消失。

## 匿名限流与运维边界

计数在同一个 Durable Object 中保存，按 Cloudflare 提供的 `CF-Connecting-IP` 经 HMAC 后限流。客户端提供的 `X-Relay-Ip` 会被覆盖，不保存原始 IP。默认限制：

| 范围 | 上限 |
| --- | --- |
| 单 IP 总 API 请求 | 每 10 分钟 60 次 |
| 全服务总 API 请求 | 每天 5000 次 |
| 单 IP 新反馈 | 每小时 3 次、每天 8 次 |
| 全服务新反馈 | 每天 200 次 |
| 单议题补充回复 | 每天 20 次 |
| 单 IP 补充回复 | 每天 60 次 |
| 全服务补充回复 | 每天 1000 次 |
| 单 IP 同时处理的 API 请求 | 2 个，超出返回 `429` |
| 全服务同时处理的 API 请求 | 16 个，超出返回 `503 service_busy` |
| 在途请求已读取正文的累计预算 | 22 MiB，超出返回 `503 service_busy` |

已存在请求的重试仍计入总请求额度，不重复占新反馈额度。共享出口的用户共享 IP 配额。分布式滥用仍可能耗尽全局额度；正式运营时可在自己的 Cloudflare 账户上补充 WAF 和预算告警，不需给客户端添加第三方反馈接口。

上传正文在业务队列外读取；新反馈及其恢复按请求编号串行，同一议题的回复、查询和回复恢复共用议题队列。不同反馈的 R2 上传和 GitHub 请求可并行；配额计数和 alarm 时间更新使用独立的短队列。alarm 跳过正在处理的记录，并在处理扫描结果前重读状态，避免覆盖已完成记录或删除在途附件。

并发名额和正文预算保留到请求结束，包括等待同一反馈的队列和处理附件的阶段；成功或失败后释放。触及并发上限或正文总预算时立即拒绝，并返回 `Retry-After: 5`；客户端保留原内容、凭证和请求编号后重试。正文预算按收到的字节计算，不是进程总内存上限。单次上传仍有 30 秒期限。

仍使用同一个 Durable Object（`coordinator-v1`）及原有存储记录；先保存 `sending` 再执行 GitHub POST 的顺序不变，重启或结果不明后只核对、不再次发帖。部署后应观察实际配额和延迟，再决定是否拆分协调器；不能直接更改对象名而丢失去重记录。

未配置服务地址时，客户端应明确显示“应用内反馈服务尚未启用”，保留表单，并提供 [`GitHub 手动提交`](https://github.com/youmikk/X-Battery/issues/new/choose)。浏览器方式由用户自己的 GitHub 登录完成；不能将本服务不可用时的请求转发到 MyFinder 或其他无关服务。

# 凭据迁移到系统钥匙串：可执行方案与测试计划

> 设计文档，未改任何代码。目标是把方案细化到"照着就能分阶段实施、每阶段可回滚、全程不碰真实库 `~/oviraptor`、绝不让任何既有安装丢失自己的密钥"。实施前需你先拍板 §8 的开放问题(尤其是否接受新依赖、是否统一由 Rust 托管密钥)。

## 0. 一句话结论（本次调研的关键发现）

不是"把几个 DB 列搬进钥匙串"那么简单——**存在两个独立的明文密钥存储**,其中最敏感的(OAuth 刷新令牌)在 Node 一侧,天真的"迁移 DB 列"方案会**整个漏掉它**:

1. **Rust 侧**:LLM 提供方 API Key 存在设置表(`app_settings` / `config_profiles`,键名 `modelApiKey` / `apiKey` / `localApiKey`,由 `model_configuration.rs` 的 `profile_setting`/`profile_value` 解析);另有 `worker_nodes.access_token`(应用自生成的工作节点令牌)。
2. **Node agent-host 侧**:`ProviderStore` 写 `<dataDir>/secrets/model-provider.json`(0600),含 `apiKey` 与 **OAuth `accessToken`/`refreshToken`**(`packages/agent-host/src/provider.mjs:135`)。刷新令牌长期有效、最敏感。

因此迁移横跨 Rust 与 Node 两个运行时,**必须先决定密钥的"唯一所有者"**(§8 问题一)。

## 1. 范围：什么是"用户密钥"，什么不是

**纳入(真正的用户机密):**

| 机密 | 位置 | 来源 | 敏感度 |
|---|---|---|---|
| LLM 提供方 API Key | `app_settings`/`config_profiles`(`modelApiKey`/`apiKey`/`localApiKey`) | 用户在配置中心输入 | 高 |
| Grok OAuth access/refresh token | Node `secrets/model-provider.json` | OAuth 流程(Node 侧) | **最高**(刷新令牌长期有效) |
| Node 侧按 provider+baseUrl 存的 apiKey | Node `secrets/model-provider.json` 的 `keys` | 同 LLM Key | 高 |
| `worker_nodes.access_token` | DB 列 | 应用 `generate_access_token()` 自生成 | 中(仅本机他进程可读库才有威胁) |

**明确排除(不是用户机密,不得迁移):**

- `investigation_validations/_api_models/_identity_diffs.api_key`(schema 416/501/565):这些是**扫描发现**——在目标应用里观测到的 API Key,是每次扫描的证据,按 `scan_id` 关联、数量多、生命周期随扫描。属于业务数据,**留在库里**。
- `*_scans`/`*_jobs` 行上的 `worker_token`(schema 868/961):内部每轮协调令牌,随心跳、临时生成,搬钥匙串无意义且有害。
- `*_tokens` 计数列(预算/用量):非机密。

## 2. 现状数据流

- **Rust 取 LLM Key**:`model_configuration.rs` → `profile_setting("apiKey","modelApiKey")` / `profile_value("localApiKey",…)` 读设置表,组装 `ModelConnection.api_key`,交给 `llm_hook_proxy` 按会话注入上游请求。
- **Node 取凭据**:`ProviderStore.read()` 从 `model-provider.json` 还原 `apiKey` 或 OAuth 令牌;`provider.mjs` 也接受调用方传入的 `apiKey`。即 Node **既能自存、也能接收**密钥。
- 两侧目前**各自独立持有明文**;没有单一所有者。

## 3. 设计

### 3.1 先决:统一所有者（建议方案 A）

- **方案 A（推荐,Rust 唯一所有者）**:所有机密由 Rust 的 `SecretStore` 托管(钥匙串优先、DB/文件回退);Node **不再自存**,每轮由 Rust 经现有已鉴权通道把需要的密钥下发给 host(`provider.mjs` 本就接受 `apiKey` 入参,改动面小)。OAuth 是难点:登录流在 Node 侧跑、拿到 refresh token——方案 A 下 host 完成 OAuth 后**把令牌回交 Rust 保管**,下轮再下发。好处:单一存储、单一钥匙串集成点、Node 零长期落盘。
- **方案 B(双钥匙串)**:Rust 和 Node 各自接钥匙串(Node 用原生 keychain 绑定)。实现面更大、两套平台后端、两处失败模式,不推荐。

下文按**方案 A**展开。

### 3.2 `SecretStore` 抽象（Rust）

```
trait SecretBackend { fn get(&self, id:&SecretId)->Result<Option<Secret>>; fn set(&self,…); fn delete(&self,…); fn available(&self)->bool; }
```

- 后端:`KeychainBackend`(`keyring` crate,服务名固定如 `com.oviraptor.desktop`,account = 逻辑密钥 id)、`DbBackend`(现有设置表/列,作为回退与迁移来源)。
- `SecretId`:稳定逻辑名,**按 profile 维度**,如 `llm.apiKey@<profileId>`、`llm.localApiKey@<profileId>`、`oauth.xai@<profileId>`、`worker.accessToken`。避免全局单键导致多 profile 互相覆盖(§8 问题四)。
- `Secret` 包裹 `zeroize` 清零的字节,Drop 时抹除(与 `sealing.rs::Key` 一致的内存卫生)。

### 3.3 可用性探测与降级（关键安全属性）

- 启动时一次性探测钥匙串可用性(写入并读回一个 `__probe__`),结果缓存。
- **任何**凭据解析失败都不得硬失败成"读不到密钥";钥匙串不可用(无头 Linux 无 Secret Service、CI、钥匙串被锁)时**自动回退 DB/文件明文**。这是"绝不把用户锁在密钥外"的硬保证。

### 3.4 读/写语义（附加式、惰性迁移）

- **读**:钥匙串命中 → 用之;未命中 → 读 DB/文件明文,若钥匙串可用则**写穿**到钥匙串(惰性迁移),返回值不变。
- **写**(用户在配置中心改 Key / OAuth 刷新):写钥匙串成功后,DB/文件只存**非机密引用/哨兵**(如 `{"stored":"keychain","id":"llm.apiKey@p1"}`),不再存明文。
- **DB/文件明文何时抹除**:**默认不抹**(双存,见 §6 回滚安全)。仅当用户显式执行"加固存储"(P3)且**钥匙串写入+读回校验通过后**,才清空明文副本,并提示"此后密钥仅存于钥匙串"。

### 3.5 平台

- macOS(本机):Keychain;首次访问弹系统授权框,用稳定 service/account 以便"始终允许";需在 UI 说明一次。
- Windows:Credential Manager。
- Linux:Secret Service(libsecret)——**服务器常常没有**,必须走 §3.3 回退,不得报错退出。

## 4. 分阶段实施（每阶段独立、flag 门控、默认 OFF 直到验证）

- **P0 地基**:引入 `keyring` 依赖 + `SecretStore` + 可用性探测;读写仍走旧路径,钥匙串读穿**默认关**(env/设置 flag)。零行为变化。
- **P1 写穿**:开 flag 后,新写入走"钥匙串 + DB 引用";读优先钥匙串、回退明文。明文副本**保留**。
- **P2 惰性迁移**:读到旧明文时写穿钥匙串(校验读回),明文副本仍**保留**(双存)。
- **P3 Node 并入(方案 A)**:host OAuth 完成后把令牌回交 Rust;Node 停止 `model-provider.json` 落盘,改为每轮接收。需配套 Node 测试。
- **P4(可选,用户显式触发)"加固"**:校验通过后清空 DB/文件明文副本,密钥仅存钥匙串。

每阶段可单独回滚:flag 关 = 回到上一阶段行为。

## 5. 测试计划（全部在隔离副本上，永不碰 `~/oviraptor`）

前置:把真实库**复制**到 scratchpad,并把其中机密**置换成合成值**后再测;OAuth 用合成令牌;LLM 用环回合成模型(复用现有 fixture)。

- **单元(Rust)**:`SecretStore` 每后端 round-trip;钥匙串不可用时回退;读穿优先级;惰性迁移"仅在钥匙串写入+读回成功后"才动明文;`Secret` Drop 清零。用 mock keychain 后端,不依赖真实系统钥匙串。
- **集成(隔离 DB 副本,合成凭据)**:① 旧明文 Key 仍能解析(回读)② 开 flag 后迁移,再读走钥匙串 ③ 关 flag(降级)仍能读 ④ profile 维度 id 不串。
- **E2E(真实进程)**:真实 bridge + 环回合成模型 + 在**测试专用 service 名**下放合成 Key;验证模型调用成功且 Key 来源为钥匙串;验证 Node host 经既有通道拿到 Key(P3 前行为不变);验证脱敏仍生效。
- **Node(P3)**:host 完成(合成)OAuth 后令牌回交 Rust;host 不落盘 `model-provider.json`;重启后仍可用(密钥从 Rust 下发)。
- **平台矩阵**:macOS 交互(本机);Linux 无 Secret Service 必须干净回退;Windows 记录待测。
- **验收**:① 任一既有安装都不会因迁移丢失密钥 ② 钥匙串关 == 今日逐字节同行为 ③ 全程真实库零改动 ④ 失败模式全部是"降级到旧路径"而非"读不到密钥"。

## 6. 回滚与安全

- flag 关即恢复纯 DB/文件行为。
- **默认双存**(P1–P3 不抹明文):即便钥匙串被清/损坏,明文副本仍在,密钥不丢——回滚天然安全。
- 仅 P4(用户显式"加固")后密钥单存钥匙串;须前置读回校验 + 明确警示,并提供"重新输入/导出"兜底。

## 7. 依赖与风险

- 新增 `keyring`(v3)直接依赖 → `Cargo.toml`/`Cargo.lock` 变动(破 `--locked`,需一次非 locked 更新);引入各平台后端。
- macOS 首次授权框 = 行为变化,需 UI 说明。
- Linux Secret Service 缺失很常见 → 回退路径是必备而非可选。
- 两存储/两运行时 → 若选方案 B 复杂度翻倍;方案 A 把复杂度集中到"OAuth 令牌回交 Rust"这一处。
- 多 profile 的密钥 id 设计若做错会互相覆盖。

## 8. 需你拍板的开放问题

1. **所有者**:采用方案 A(Rust 唯一托管,Node 不再落盘)还是 B(两侧各接钥匙串)?(建议 A)
2. **验证后默认开/关**:钥匙串读穿在验证通过后默认 ON 还是保持 OFF 由用户在设置里开?(建议先 OFF,给一版观察)
3. **是否允许单存**:是否要 P4"加固"(抹明文)?还是永远双存(更安全、但明文仍在库里)?
4. **密钥 id 粒度**:按 profile(`…@<profileId>`)还是全局单键?(建议按 profile)
5. **是否接受 `keyring` 依赖**与由此带来的 `Cargo.lock` 变动。

## 9. 不做什么(明确边界)

- 不迁移 `investigation_*.api_key`(扫描发现)与 `worker_token`(内部临时令牌)。
- 实施前不碰真实库;所有验证在隔离副本 + 合成凭据上完成。
- 未经你在 §8 拍板前不动代码、不加依赖。

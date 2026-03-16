# csimulator

`csimulator` 是 cat-chip 项目第一阶段的 Rust 原型工程，对应主规格文档中的“可表示、可编译、可调度、可验证”闭环。

它实现了以下主链路：

`BlockMat IR -> optimize -> lower -> schedule -> simulate -> oracle -> report`

当前目标不是 RTL 或生产级硬件微架构，而是验证：

- 固定基础块 `B = k^2`
- `BlockMat(n, m, K)` 作为唯一 IR
- R1-R8 中第一阶段需要的核心结构识别与代价建模
- 周期级调度与数值模拟闭环
- 与 FP32 oracle 的结果对照

## 目录结构

`src/` 下的主要模块：

- `ir.rs`: `BlockMat`、`KernelRef`、维度与合法性检查
- `semantics.rs`: `eval`、`compose`、显式态射加法 `morph_add`、态射数乘 `morph_scale`、结构构造函数 `proj/inj/pair/copair/copy/merge/bimap`
- `optimize.rs`: R1/R2/R3 识别、结构统计、参数共享统计
- `lower.rs`: `BlockMat -> SchedGraph` lowering，含共享参数句柄复用
- `schedule.rs`: 周期级调度器，建模 core/bcast/reduce 资源占用
- `simulate.rs`: 按 schedule 推进数值状态与缓冲状态，输出逐周期 trace
- `report.rs`: 解析代价、调度跨度、资源峰值报告
- `oracle.rs`: 模拟输出与 `eval()` 路径的端到端对照
- `trace.rs`: `schedule` / `sim trace` / `report` 的 JSON 导出
- `benchmarks.rs`: V1-V8 代表性验证程序
- `main.rs`: CLI 入口

## 已实现范围

当前实现已经覆盖第一阶段原型的核心要求：

- `BlockMat` 作为唯一 IR
- eager flattening 的 `compose`
- 由双积结构诱导的显式态射加法 `morph_add = ∇ ∘ (f ⊕ g) ∘ Δ`
- Hom 集上的显式标量作用 `morph_scale(c, f)`
- `Id` / `Zero` / `Scalar(c * I)` 的识别与执行分流
- 参数共享从统计层进入 lowering，复用 `param_handle`
- Broadcast / Core / Reduce / Direct / ZeroFill 的调度图建模
- Direct 节点走零资源路径
- schedule 与 simulate 分离
- 逐周期 `sim_trace` 导出
- 解析代价与实际调度周期对照
- V1-V8 benchmark 自动化测试
- `morph_add` 驱动的双积分解实验案例

## 当前边界

这个工程仍然是第一阶段原型，不是 RTL 级实现。当前没有覆盖：

- 真实硬件接口编码
- 端口仲裁细节与背压协议
- 更细的双缓冲显式时序模型
- SRAM/面积/功耗建模
- `B = k^d` 中 `d > 2` 的通用化实现

## 构建与测试

在 `simulator/` 目录下执行：

```bash
cargo test -- --nocapture
```

当前测试覆盖：

- IR 合法性检查
- 结构构造函数边界检查
- S1-S6 关键结构等式
- 态射加法的零元、结合律、交换律、与 compose 的双侧分配律
- Hom 集线性结构中的标量单位元、零元与分配律
- 双积关键分解等式 `i0∘p0 + i1∘p1 = id`
- 参数共享句柄复用
- 广播延迟按 fanout 计算
- Direct 不占用 core 资源
- 单项行写回与临时缓冲释放
- V1-V8 端到端闭环

## CLI 用法

列出所有 benchmark：

```bash
cargo run -- list
```

运行默认示例（V1）：

```bash
cargo run -- V1
```

运行指定示例并导出 JSON：

```bash
cargo run -- V6 artifacts/v6
```

通用格式：

```bash
cargo run -- [CASE_ID] [OUTPUT_DIR]
```

其中：

- `CASE_ID` 可选，支持 `V1` 到 `V8`，默认 `V1`
- `OUTPUT_DIR` 可选，默认写到 `artifacts/<case_id_lowercase>/`

## Benchmark 集合

`benchmarks.rs` 中内置了八个案例：

- `V1`: 全稠密基准
- `V2`: 含零块，验证 R1
- `V3`: 含恒等块，验证 R2
- `V4`: 块对角，验证单项行特化
- `V5`: 置换矩阵，验证纯结构零执行路径
- `V6`: 大规模混合，验证完整闭环与代价对比
- `V7`: 双积分解加法，验证 `morph_add` 组装局部态射后的端到端闭环
- `V8`: 128 维多结构混合，在单一案例中联合覆盖 R1-R7 的主要结构收益场景，R8 仍由 V5 单独验证

## 导出文件

每次通过 CLI 运行一个 case，都会在输出目录中生成：

- `schedule.json`: 调度结果，含 `topo_order`、`batches`、`timed_nodes`
- `sim_trace.json`: 逐周期模拟轨迹，含活跃节点、周期末占用与周期内瞬时峰值占用
- `report.json`: 结构统计、解析代价、调度跨度、资源峰值、展平基线

仓库还提供了一个零依赖 Python 可视化脚本：

```bash
python3 scripts/visualize_artifacts.py artifacts/v7
```

默认会在对应 artifact 目录下生成 `viz/` 子目录，包含：

- `schedule_gantt.svg`: 调度甘特图
- `buffer_occupancy.svg`: 输入/临时/输出缓冲占用曲线
- `index.html`: 汇总页面

这些文件可直接用于：

- 画甘特图或调度时序图
- 画缓冲占用曲线，并区分“周期末占用”和“周期内瞬时峰值”
- 做 spec 验收图表
- 对比 block 路线与 flat baseline

## 一次运行的输出内容

CLI 标准输出会显示：

- 结构统计 `StructStats`
- 调度图节点数
- schedule 批次数与总周期
- 输出块结果
- `peak_temp_buffer`
- oracle 对照结果
- 代价报告摘要

其中代价报告同时给出：

- 解析上界 `C_total_up`
- 双缓冲有效值 `C_total_eff`
- 实际调度跨度与总周期
- 资源峰值：`peak temp`、`max core`、`max bcast`、`max reduce`
- 展平基线 `C_flat`
- 代价比 `R = block / flat`

## 与规格文档的关系

建议结合以下文档一起看：

- `../技术规格说明-0v2.md`
- `../模拟器技术选型说明-0v1.md`

本工程实现的定位是：

- 对主规格文档第一阶段要求做可运行验证
- 为后续可视化、benchmark 收集和架构迭代提供实验底座

如果要继续往前推进，下一步通常是：

- 批量导出 V1-V8 的 artifacts
- 增加可视化脚本
- 把调度/缓冲 trace 接到文档验收图

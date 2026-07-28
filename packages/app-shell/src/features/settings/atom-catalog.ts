import type { Agent, Skill } from "@ora/contracts";

/**
 * Catalog identifiers keep bundled records editable in memory while persisted
 * records continue through the backend CRUD path.
 */
export const CATALOG_ATOM_ID_PREFIX = "catalog-";

export const CATALOG_AGENTS = [
  { id: "catalog-role-internal-storage-architect", name: "存储系统架构师", description: "设计块、文件与对象存储的数据路径、可靠性边界和演进方案。" },
  { id: "catalog-role-code-reviewer", name: "代码审查专家", description: "按正确性、可维护性和安全性审查变更，并给出可直接执行的修改建议。" },
  { id: "catalog-role-distributed-storage-engineer", name: "分布式存储工程师", description: "处理副本、一致性、故障恢复和容量扩展等存储核心问题。" },
  { id: "catalog-role-rust-systems-engineer", name: "Rust 系统工程师", description: "以类型安全、并发正确性和可测试性实现高性能系统组件。" },
  { id: "catalog-role-internal-storage-validator", name: "存储验证专家", description: "规划数据完整性、故障注入、升级兼容与长稳测试体系。" },
  { id: "catalog-role-kubernetes-platform-engineer", name: "Kubernetes 平台工程师", description: "设计控制器、资源模型和可靠的云原生交付能力。" },
  { id: "catalog-role-linux-kernel-engineer", name: "Linux 内核工程师", description: "分析内核、I/O、内存与驱动路径中的性能和正确性问题。" },
  { id: "catalog-role-object-storage-engineer", name: "对象存储工程师", description: "围绕 S3 兼容接口、元数据、分片与数据耐久性开发服务。" },
  { id: "catalog-role-frontend-engineer", name: "React 前端工程师", description: "使用 React 与 TypeScript 构建可访问、可测试的复杂工程界面。" },
  { id: "catalog-role-database-kernel-engineer", name: "数据库内核工程师", description: "优化事务、日志、索引和存储引擎的关键执行路径。" },
  { id: "catalog-role-ceph-maintainer", name: "Ceph 维护专家", description: "诊断 OSD、MON、PG 与 CRUSH 拓扑相关的集群问题。" },
  { id: "catalog-role-cicd-engineer", name: "CI/CD 工程师", description: "建设可重复的构建、测试、制品和渐进式发布流水线。" },
  { id: "catalog-role-block-storage-engineer", name: "块存储工程师", description: "开发卷管理、快照、复制以及低时延 I/O 数据面能力。" },
  { id: "catalog-role-typescript-engineer", name: "TypeScript 工程师", description: "收紧类型边界，设计稳定的前端状态与跨包接口契约。" },
  { id: "catalog-role-performance-engineer", name: "性能工程师", description: "通过基准、火焰图和系统指标定位延迟与吞吐瓶颈。" },
  { id: "catalog-role-sre", name: "SRE 可靠性工程师", description: "围绕 SLO、容量、可观测性与恢复时间提升服务可靠性。" },
  { id: "catalog-role-filesystem-engineer", name: "文件系统工程师", description: "处理 POSIX 语义、缓存一致性、元数据和崩溃恢复。" },
  { id: "catalog-role-security-reviewer", name: "应用安全审查员", description: "识别代码、依赖与配置中的攻击面和权限风险。" },
  { id: "catalog-role-gitlab-maintainer", name: "GitLab 平台维护者", description: "优化合并请求、Runner、流水线模板和研发协作流程。" },
  { id: "catalog-role-test-architect", name: "测试架构师", description: "设计单元、集成、系统与端到端测试的分层验证策略。" },
  { id: "catalog-role-cloud-native-architect", name: "云原生架构师", description: "规划服务边界、弹性、交付和跨环境运行模型。" },
  { id: "catalog-role-cache-engineer", name: "缓存系统工程师", description: "设计缓存一致性、淘汰策略和热点数据保护机制。" },
  { id: "catalog-role-ebpf-observability", name: "eBPF 可观测性工程师", description: "利用内核遥测定位网络、调度与 I/O 的系统级问题。" },
  { id: "catalog-role-api-designer", name: "API 设计专家", description: "定义一致、可演进且便于自动化调用的接口与错误模型。" },
  { id: "catalog-role-rocksdb-engineer", name: "RocksDB 存储引擎专家", description: "优化 LSM、压实、缓存与写放大等存储引擎路径。" },
  { id: "catalog-role-debugging-specialist", name: "复杂故障诊断专家", description: "结合日志、指标、转储和复现线索建立可验证的根因假设。" },
  { id: "catalog-role-open-source-maintainer", name: "开源项目维护者", description: "评估贡献质量、兼容策略、版本发布和社区协作影响。" },
  { id: "catalog-role-data-engineer", name: "数据工程师", description: "构建可靠的数据管道、质量规则和可追溯的数据资产。" },
  { id: "catalog-role-storage-protocol-validator", name: "存储协议验证工程师", description: "验证 NVMe、SCSI、NFS 与 SMB 等协议的兼容性和异常行为。" },
  { id: "catalog-role-react-engineer", name: "React 组件架构师", description: "设计组合友好、性能稳定且具备无障碍语义的组件系统。" },
  { id: "catalog-role-release-manager", name: "发布工程师", description: "检查变更、制品、迁移和回滚准备，保障版本稳定交付。" },
  { id: "catalog-role-go-engineer", name: "Go 后端工程师", description: "构建高并发服务、控制器和易于运维的基础设施组件。" },
  { id: "catalog-role-network-storage-engineer", name: "网络存储工程师", description: "分析 RDMA、TCP 与存储协议叠加路径中的性能和可靠性。" },
  { id: "catalog-role-python-automation", name: "Python 自动化工程师", description: "编写可靠的测试框架、数据工具和工程自动化脚本。" },
  { id: "catalog-role-chaos-engineer", name: "混沌工程师", description: "设计故障注入实验，验证降级、隔离和自动恢复能力。" },
  { id: "catalog-role-postgresql-expert", name: "PostgreSQL 专家", description: "分析事务、查询计划、复制和数据库运行稳定性。" },
  { id: "catalog-role-desktop-engineer", name: "跨平台桌面工程师", description: "处理原生能力、进程通信、升级与桌面端性能问题。" },
  { id: "catalog-role-compiler-engineer", name: "LLVM 编译器工程师", description: "诊断编译链、代码生成、优化和跨架构兼容问题。" },
  { id: "catalog-role-quality-engineer", name: "质量工程师", description: "从需求到发布建立可量化、可追踪的工程质量门禁。" },
  { id: "catalog-role-agent-architect", name: "Coding Agent 架构师", description: "设计工具调用、上下文、权限和多 Agent 协作边界。" },
  { id: "catalog-role-rag-engineer", name: "代码检索工程师", description: "优化代码切分、符号检索、重排和引用可信度。" },
  { id: "catalog-role-devops-engineer", name: "DevOps 工程师", description: "统一本地开发、自动化检查和多环境发布工作流。" },
  { id: "catalog-role-backup-recovery", name: "备份恢复工程师", description: "设计备份、恢复、时间点回滚和灾备演练方案。" },
  { id: "catalog-role-observability-engineer", name: "可观测性工程师", description: "建设日志、指标、追踪和面向故障的诊断视图。" },
  { id: "catalog-role-migration-advisor", name: "系统迁移顾问", description: "规划兼容验证、数据迁移和风险可控的分阶段切换。" },
  { id: "catalog-role-database-reliability", name: "数据库可靠性工程师", description: "保障复制、备份、容量和故障切换的长期稳定性。" },
  { id: "catalog-role-accessibility-engineer", name: "前端无障碍工程师", description: "审查键盘、语义、对比度和辅助技术兼容性。" },
  { id: "catalog-role-incident-commander", name: "故障响应指挥", description: "在生产故障中组织研判、止损、沟通与恢复行动。" },
  { id: "catalog-role-supply-chain-security", name: "软件供应链安全专家", description: "审查依赖、制品、签名和构建链路中的供应链风险。" },
  { id: "catalog-role-distributed-systems", name: "分布式系统架构师", description: "评估一致性、可用性、分区容错和数据所有权边界。" },
  { id: "catalog-role-internal-pipeline-coordinator", name: "内部流水线协调员", description: "衔接代码仓、评审、构建、验证和缺陷闭环的工程流程。" },
  { id: "catalog-role-internal-storage-integration", name: "内部存储集成工程师", description: "负责存储组件接入、接口联调和版本兼容验证。" },
  { id: "catalog-role-internal-quality-gate", name: "内部质量门禁专家", description: "将静态检查、测试覆盖和风险规则编排为准入策略。" },
  { id: "catalog-role-internal-repository-maintainer", name: "内部代码仓维护者", description: "治理分支、合并请求、权限和仓库自动化规范。" },
  { id: "catalog-role-internal-release-verifier", name: "内部版本验证工程师", description: "验证候选版本的功能、升级、回滚与环境兼容性。" },
  { id: "catalog-role-internal-storage-performance", name: "内部存储性能专家", description: "面向真实负载分析 IOPS、带宽、尾延迟和资源利用率。" },
  { id: "catalog-role-internal-failure-lab", name: "内部故障实验室专家", description: "复现介质、网络和节点异常，验证数据安全与恢复能力。" },
] satisfies Agent[];

export const CATALOG_SKILLS = [
  { id: "catalog-skill-internal-cdase-build", name: "cdase-build", description: "对接内部代码仓与研发平台，编排 MR 创建、智能 Review、流水线执行、缺陷定位及自动修复，形成可审计的代码交付闭环。" },
  { id: "catalog-skill-storage-regression", name: "存储回归测试编排", description: "按变更影响选择数据面、控制面、升级和故障恢复用例。" },
  { id: "catalog-skill-code-review", name: "代码审查", description: "按严重程度检查正确性、可维护性、安全性和测试覆盖。" },
  { id: "catalog-skill-rust-quality", name: "Rust 质量检查", description: "组合 rustfmt、Clippy、cargo test 与依赖检查验证 Rust 改动。" },
  { id: "catalog-skill-object-api-conformance", name: "对象存储 API 一致性验证", description: "验证 S3 兼容接口、错误码、并发语义和异常输入行为。" },
  { id: "catalog-skill-ci-diagnosis", name: "CI 失败诊断", description: "从流水线日志定位首个有效失败并生成最小修复方案。" },
  { id: "catalog-skill-distributed-consistency", name: "分布式一致性验证", description: "覆盖副本延迟、脑裂、重选主和网络分区下的数据一致性。" },
  { id: "catalog-skill-unit-test", name: "单元测试生成", description: "围绕公开行为、边界条件和失败路径生成可维护测试。" },
  { id: "catalog-skill-ceph-troubleshooting", name: "Ceph 集群诊断", description: "分析健康状态、PG 分布、OSD 延迟和恢复过程。" },
  { id: "catalog-skill-git-diff-summary", name: "Git 变更分析", description: "归纳改动意图、影响范围、关键依赖与潜在回归风险。" },
  { id: "catalog-skill-data-integrity", name: "数据完整性校验", description: "设计校验和、静默损坏检测与端到端数据比对流程。" },
  { id: "catalog-skill-playwright", name: "Playwright 端到端测试", description: "生成稳定的浏览器流程、定位器、断言和失败追踪。" },
  { id: "catalog-skill-internal-mr-review", name: "内部 MR 评审编排", description: "聚合差异、关联需求、检查结果和评审意见，推动合入闭环。" },
  { id: "catalog-skill-fio-benchmark", name: "FIO 性能基准", description: "设计贴近业务负载的 IOPS、带宽与尾延迟测试矩阵。" },
  { id: "catalog-skill-api-design", name: "API 契约设计", description: "生成一致、可演进且适合自动化调用的接口与错误模型。" },
  { id: "catalog-skill-kubernetes-review", name: "Kubernetes 配置审查", description: "检查资源、探针、权限、调度和高可用配置。" },
  { id: "catalog-skill-filesystem-semantics", name: "文件系统语义验证", description: "验证 POSIX 行为、并发访问、崩溃一致性和权限边界。" },
  { id: "catalog-skill-typescript-hardening", name: "TypeScript 类型加固", description: "消除隐式 any、不安全断言和跨模块类型泄漏。" },
  { id: "catalog-skill-fault-injection", name: "存储故障注入", description: "注入磁盘、网络、进程和节点故障，验证隔离与恢复机制。" },
  { id: "catalog-skill-gitlab-ci", name: "GitLab CI 优化", description: "优化 Runner、缓存、并行任务和可复用流水线模板。" },
  { id: "catalog-skill-snapshot-replication", name: "快照与复制验证", description: "检查一致性快照、增量复制、回滚和灾备切换。" },
  { id: "catalog-skill-react-performance", name: "React 性能分析", description: "定位重复渲染、状态抖动、资源加载和交互延迟。" },
  { id: "catalog-skill-cargo-nextest", name: "cargo-nextest 测试加速", description: "配置并行执行、重试、分区和失败归档策略。" },
  { id: "catalog-skill-nvme-validation", name: "NVMe 协议验证", description: "检查命令集、队列、超时、复位和异常完成路径。" },
  { id: "catalog-skill-debugging", name: "复杂故障定位", description: "根据日志、指标、转储和复现条件建立根因证据链。" },
  { id: "catalog-skill-dockerfile-review", name: "Dockerfile 审查", description: "优化镜像安全、体积、缓存和运行时权限。" },
  { id: "catalog-skill-erasure-coding", name: "纠删码可靠性验证", description: "验证编码、降级读、重建和多故障组合下的数据安全。" },
  { id: "catalog-skill-pytest", name: "pytest 测试设计", description: "生成 fixture、参数化、隔离和清晰的失败断言。" },
  { id: "catalog-skill-storage-upgrade", name: "存储升级兼容验证", description: "覆盖滚动升级、混部版本、数据迁移和回滚路径。" },
  { id: "catalog-skill-commit-message", name: "Conventional Commit 生成", description: "根据实际变更生成准确、简洁且符合规范的提交信息。" },
  { id: "catalog-skill-ebpf-analysis", name: "eBPF I/O 路径分析", description: "追踪系统调用、块层、网络与调度热点。" },
  { id: "catalog-skill-terraform-review", name: "Terraform 变更审查", description: "评估基础设施变更、权限、漂移与回滚风险。" },
  { id: "catalog-skill-internal-pipeline-repair", name: "内部流水线自动修复", description: "识别可确定修复的构建与测试失败，生成补丁并触发复验。" },
  { id: "catalog-skill-sql-optimization", name: "SQL 执行计划优化", description: "分析索引、数据分布和执行算子以降低查询成本。" },
  { id: "catalog-skill-nfs-smb", name: "NFS/SMB 兼容验证", description: "验证共享、锁、权限、缓存和断线重连行为。" },
  { id: "catalog-skill-eslint", name: "ESLint 规则治理", description: "统一代码约束，减少无效规则与跨包配置漂移。" },
  { id: "catalog-skill-chaos-testing", name: "混沌测试设计", description: "规划可控故障、稳态指标、停止条件和恢复验证。" },
  { id: "catalog-skill-release-readiness", name: "发布准备检查", description: "核对制品、迁移、测试、风险、监控和回滚条件。" },
  { id: "catalog-skill-sanitizer", name: "Sanitizer 内存诊断", description: "利用 ASan、TSan 与 UBSan 定位内存和并发缺陷。" },
  { id: "catalog-skill-internal-storage-gate", name: "内部存储准入检查", description: "聚合协议、性能、可靠性和升级结果形成版本准入结论。" },
  { id: "catalog-skill-ruff", name: "Ruff Python 质量检查", description: "统一 Python lint、格式化和导入规则并修复常见问题。" },
  { id: "catalog-skill-capacity-model", name: "存储容量模型", description: "评估有效容量、副本或纠删码开销、增长和水位风险。" },
  { id: "catalog-skill-security-audit", name: "代码安全审计", description: "识别注入、越权、敏感数据和不安全依赖风险。" },
  { id: "catalog-skill-helm-review", name: "Helm Chart 审查", description: "检查模板、默认值、升级兼容和环境覆盖策略。" },
  { id: "catalog-skill-storage-metrics", name: "存储可观测性设计", description: "定义容量、时延、错误、恢复和数据健康指标。" },
  { id: "catalog-skill-github-actions", name: "GitHub Actions 优化", description: "改进矩阵构建、缓存、权限和可复用工作流。" },
  { id: "catalog-skill-api-fuzzing", name: "API 模糊测试", description: "生成边界、畸形和状态组合输入以发现健壮性缺陷。" },
  { id: "catalog-skill-architecture-decision", name: "架构决策记录", description: "整理背景、约束、备选方案、权衡和最终决定。" },
  { id: "catalog-skill-internal-defect-loop", name: "内部缺陷闭环", description: "关联缺陷、代码修改、评审与验证证据，跟踪直至关闭。" },
  { id: "catalog-skill-postgresql", name: "PostgreSQL 诊断", description: "分析锁、慢查询、复制延迟与参数配置问题。" },
  { id: "catalog-skill-backup-restore", name: "备份恢复演练", description: "验证备份可用性、恢复时间、数据点和操作手册。" },
  { id: "catalog-skill-dependency-audit", name: "依赖与许可证审计", description: "检查漏洞、许可证、版本漂移和供应链风险。" },
  { id: "catalog-skill-controller-test", name: "Kubernetes 控制器测试", description: "验证协调循环、幂等性、终态与异常重试行为。" },
  { id: "catalog-skill-log-analysis", name: "日志关联分析", description: "从多组件日志中提取异常模式、时间线和因果线索。" },
  { id: "catalog-skill-storage-migration", name: "存储数据迁移验证", description: "检查数据一致性、增量同步、切换和回退流程。" },
  { id: "catalog-skill-frontend-accessibility", name: "前端无障碍检查", description: "检查键盘、语义、焦点、对比度和读屏体验。" },
  { id: "catalog-skill-semver", name: "语义化版本评估", description: "根据 API 与行为变化判断版本级别和兼容影响。" },
  { id: "catalog-skill-prompt-injection", name: "Agent 提示注入审查", description: "检查外部内容劫持指令、上下文和工具调用的风险。" },
  { id: "catalog-skill-internal-environment", name: "内部环境一致性验证", description: "比对依赖、配置、制品和数据准备，减少环境型失败。" },
  { id: "catalog-skill-performance-regression", name: "性能回归分析", description: "对比基线、识别显著退化并定位关联代码路径。" },
  { id: "catalog-skill-contract-test", name: "接口契约测试", description: "验证服务边界、版本兼容、错误语义和消费者预期。" },
  { id: "catalog-skill-release-notes", name: "发布说明生成", description: "将技术改动转化为清晰的升级影响与用户可见内容。" },
  { id: "catalog-skill-flamegraph", name: "火焰图性能分析", description: "识别 CPU 热点、锁竞争与非预期调用路径。" },
] satisfies Skill[];

export const COMMON_AGENT_IDS = [
  "catalog-role-internal-storage-architect",
  "catalog-role-code-reviewer",
  "catalog-role-rust-systems-engineer",
  "catalog-role-debugging-specialist",
] as const;

export const COMMON_SKILL_IDS = [
  "catalog-skill-internal-cdase-build",
  "catalog-skill-code-review",
  "catalog-skill-storage-regression",
  "catalog-skill-ci-diagnosis",
] as const;

export type SkillMarketCategory = "build" | "cloud" | "observability" | "security" | "diagnostics";

/** Adds marketplace presentation metadata without leaking it into the persisted skill contract. */
export interface SkillMarketItem extends Skill {
  category: SkillMarketCategory;
  publisher: string;
  featured?: boolean;
}

export const SKILL_MARKET_ITEMS = [
  { id: "catalog-market-bazel", name: "Bazel 构建分析", description: "诊断依赖图、远程缓存和增量构建性能。", category: "build", publisher: "Bazel Community", featured: true },
  { id: "catalog-market-nix", name: "Nix 可复现环境", description: "生成可复现的开发环境与构建定义。", category: "build", publisher: "NixOS Foundation" },
  { id: "catalog-market-argocd", name: "Argo CD 漂移检查", description: "识别 GitOps 期望状态与集群状态差异。", category: "cloud", publisher: "CNCF" },
  { id: "catalog-market-otel", name: "OpenTelemetry 接入", description: "设计跨服务追踪、指标和上下文传播。", category: "observability", publisher: "CNCF", featured: true },
  { id: "catalog-market-prometheus", name: "Prometheus 告警审查", description: "优化指标查询、阈值和告警可行动性。", category: "observability", publisher: "CNCF" },
  { id: "catalog-market-valgrind", name: "Valgrind 内存分析", description: "定位泄漏、越界访问和未初始化数据。", category: "diagnostics", publisher: "Valgrind Developers" },
  { id: "catalog-market-trivy", name: "Trivy 制品扫描", description: "检查镜像、文件系统和 IaC 安全问题。", category: "security", publisher: "Aqua Security" },
] satisfies SkillMarketItem[];

export const INTERNAL_STORAGE_SKILL_IDS = [
  "catalog-skill-storage-regression",
  "catalog-skill-object-api-conformance",
  "catalog-skill-distributed-consistency",
  "catalog-skill-ceph-troubleshooting",
  "catalog-skill-data-integrity",
  "catalog-skill-fio-benchmark",
  "catalog-skill-filesystem-semantics",
  "catalog-skill-fault-injection",
  "catalog-skill-snapshot-replication",
  "catalog-skill-nvme-validation",
  "catalog-skill-erasure-coding",
  "catalog-skill-storage-upgrade",
  "catalog-skill-ebpf-analysis",
  "catalog-skill-nfs-smb",
  "catalog-skill-internal-storage-gate",
  "catalog-skill-capacity-model",
  "catalog-skill-storage-metrics",
  "catalog-skill-backup-restore",
  "catalog-skill-storage-migration",
] as const;

const INTERNAL_STORAGE_SKILL_ID_SET = new Set<string>(INTERNAL_STORAGE_SKILL_IDS);

/** Distinguishes bundled catalog records from persisted records without extending API payloads. */
export function isCatalogAtom(item: Agent | Skill): boolean {
  return item.id.startsWith(CATALOG_ATOM_ID_PREFIX);
}

/** Identifies entries that represent organization-specific engineering workflows. */
export function isInternalAtom(item: Agent | Skill): boolean {
  return isCatalogAtom(item)
    && (item.id.includes("-internal-") || INTERNAL_STORAGE_SKILL_ID_SET.has(item.id));
}

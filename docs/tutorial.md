# sradb-rs 使用教程

`sradb-rs` 是一个用于查询和下载 NGS 元数据与数据的命令行工具，覆盖常见的 NCBI SRA、ENA 和 GEO 工作流。它可以获取 SRA 元数据、在 SRP/SRX/SRR/SRS/GSE/GSM 之间转换编号、搜索 SRA、下载 ENA FASTQ、下载 GEO Series Matrix，并从 PMID/DOI/PMC 文献编号中提取数据库编号。

## 安装

在仓库根目录执行：

```bash
cargo install --path crates/sradb-cli
```

安装完成后确认命令可用：

```bash
sradb info
sradb --help
```

本项目要求 Rust 1.80 或更高版本。开发时也可以不安装，直接从仓库运行：

```bash
cargo run -p sradb-cli -- info
```

## 推荐配置

大多数命令会访问 NCBI、ENA 或 GEO 的在线服务。推荐配置 NCBI 邮箱和 API key：

```bash
export NCBI_EMAIL="you@example.com"
export NCBI_API_KEY="your_ncbi_api_key"
```

`NCBI_API_KEY` 不是必需的，但能把 NCBI eUtils 请求限速从 3 rps 提高到 10 rps。

只有使用 `metadata --enrich` 时才需要 OpenAI 兼容接口：

```bash
export OPENAI_API_KEY="sk-..."
export OPENAI_MODEL="gpt-4o-mini"
export OPENAI_BASE_URL="https://api.openai.com"
```

`OPENAI_BASE_URL` 可以替换为其他 OpenAI 兼容服务，例如 Azure、Together、vLLM、llama.cpp server 或 Ollama 的 `/v1` endpoint。

## 快速开始

查询一个 SRA study 的默认元数据：

```bash
sradb metadata SRP174132
```

默认输出 TSV。常用列包括 study、experiment、sample、run、organism、library、platform 和 run size 等信息。

保存为文件：

```bash
sradb metadata SRP174132 --format tsv > SRP174132.metadata.tsv
```

输出 JSON：

```bash
sradb metadata SRP174132 --format json > SRP174132.metadata.json
```

输出 NDJSON，适合流式处理：

```bash
sradb metadata SRP174132 --format ndjson > SRP174132.metadata.ndjson
```

## 获取详细元数据

默认元数据偏轻量。如果需要样本属性、ENA FASTQ URL、NCBI/S3/GS 下载链接，使用 `--detailed`：

```bash
sradb metadata SRP174132 --detailed --format tsv > SRP174132.detailed.tsv
```

详细 TSV 会额外包含这些固定列：

- `ena_fastq_http_1`
- `ena_fastq_http_2`
- `ena_fastq_ftp_1`
- `ena_fastq_ftp_2`
- `ncbi_url`
- `s3_url`
- `gs_url`

它还会把样本属性展开为动态列，列名形如 `sample_attribute_<key>`。因为动态列来自所有结果行的 sample attributes 并集，不同项目的列数可能不同。

一次查询多个 accession：

```bash
sradb metadata SRP174132 GSE56924 --detailed --format json
```

`metadata` 支持常见 SRA/GEO accession，例如 `SRP`、`SRX`、`SRR`、`SRS`、`GSE`、`GSM`。

## 用 LLM 补充生物学字段

`--enrich` 会根据样本标题、实验标题和样本属性提取结构化生物学字段：

- `organ`
- `tissue`
- `anatomical_system`
- `cell_type`
- `disease`
- `sex`
- `development_stage`
- `assay`
- `organism`

示例：

```bash
export OPENAI_API_KEY="sk-..."
sradb metadata SRP174132 --detailed --enrich --format json > SRP174132.enriched.json
```

建议同时使用 `--detailed`，因为 sample attributes 越完整，LLM 可提取的信息越稳定。`--enrich` 会调用外部模型服务，处理大项目时应预估调用量和费用。

## 转换 accession

`convert` 的格式是：

```bash
sradb convert <FROM> <TO> <ACCESSION>...
```

`FROM` 和 `TO` 可选值：

- `srp`
- `srx`
- `srr`
- `srs`
- `gse`
- `gsm`

常见示例：

```bash
sradb convert srp srx SRP174132
sradb convert srp srr SRP174132
sradb convert srr srp SRR8361601
sradb convert gse srp GSE56924
sradb convert gse gsm GSE56924
sradb convert gsm srp GSM1371490
```

输出为两列 TSV：

```text
<input_accession>    <converted_accession>
```

例如：

```bash
sradb convert srp srr SRP174132 > SRP174132.runs.tsv
```

支持的转换包含常见的 SRA 家族互转、GSE/GSM 与 SRA 的映射，以及部分链式转换，例如 `gse -> srx/srr/srs` 会先通过 `srp` 转换。

## 搜索 SRA

`search` 使用 NCBI Entrez 查询 SRA，并返回匹配 run 的元数据。

按物种和建库策略搜索：

```bash
sradb search --organism "Homo sapiens" --strategy RNA-Seq --max 10
```

输出 JSON：

```bash
sradb search \
  --organism "Homo sapiens" \
  --strategy RNA-Seq \
  --platform ILLUMINA \
  --max 20 \
  --format json
```

可用过滤条件：

- `--query`：自由文本
- `--organism`：物种名，例如 `"Homo sapiens"`
- `--strategy`：library strategy，例如 `RNA-Seq`、`ChIP-Seq`、`WGS`
- `--source`：library source，例如 `TRANSCRIPTOMIC`、`GENOMIC`
- `--selection`：library selection，例如 `cDNA`、`ChIP`
- `--layout`：`SINGLE` 或 `PAIRED`
- `--platform`：测序平台，例如 `ILLUMINA`、`OXFORD_NANOPORE`
- `--max`：返回结果数，默认 20，单次请求最多 500
- `--format`：`tsv`、`json` 或 `ndjson`

`search` 至少需要一个过滤条件或自由文本查询。空查询会报错。

## 下载 SRA 或 FASTQ

`download` 会先获取详细元数据，再按下载源生成下载计划。默认下载源是 `ncbi`，会下载 NCBI 提供的 SRA / SRA Lite 文件；如果需要 ENA/EBI 的 FASTQ 文件，可以显式指定 `--source ena`。

从 NCBI 下载一个 study：

```bash
sradb download SRP174132 --out-dir ./sra -j 4
```

从 ENA/EBI 下载 FASTQ：

```bash
sradb download SRP174132 --source ena --out-dir ./fastq -j 4
```

参数说明：

- `--source`：下载源，`ncbi` 或 `ena`，默认 `ncbi`
- `--out-dir`：输出目录，默认 `./sradb_downloads`
- `-j, --parallelism`：并行下载 worker 数，默认 4

下载过程会显示一个总览行，以及每个文件各自的进度行。每个文件会单独显示字节进度、速度、ETA 和重试状态；如果输出目录里已经有 `.part` 文件，下一次运行会自动从已有字节继续下载，不需要删除临时文件。

文件会按 study 和 experiment 分目录保存。NCBI 示例：

```text
sra/
  SRP174132/
    SRX5172107/
      SRR8361601.sralite.1
```

ENA FASTQ 示例：

```text
fastq/
  SRP174132/
    SRX5172107/
      SRR8361601_1.fastq.gz
      SRR8361601_2.fastq.gz
```

如果某个 accession 没有对应下载源的 URL，命令会提示缺失的来源并返回非零退出码。可以换一个来源重试，例如 `--source ena` 或 `--source ncbi`。

## 下载 GEO Series Matrix

下载 GEO Series Matrix 原始 `.txt.gz`：

```bash
sradb geo matrix GSE56924 --out-dir ./geo
```

同时解析出矩阵 TSV：

```bash
sradb geo matrix GSE56924 --out-dir ./geo --parse-tsv
```

输出文件：

```text
geo/
  GSE56924_series_matrix.txt.gz
  GSE56924_series_matrix.tsv
```

`--parse-tsv` 只保留 series matrix table 的 header 和数据表。Series/Sample metadata 会被解析用于统计，但 CLI 当前不会单独写出 metadata 文件。

## 从文献编号提取数据库编号

`id` 支持 PMID、PMC 和 DOI：

```bash
sradb id 39528918
sradb id PMC10802650
sradb id 10.12688/f1000research.18676.1
```

默认输出为纯文本键值行：

```text
pmid:   39528918
pmc:    PMC10802650
gse:    GSE...
srp:    SRP...
prjna:  PRJNA...
```

输出 JSON：

```bash
sradb id 39528918 --json
```

这个命令适合从论文出发，快速找到关联的 `GSE`、`GSM`、`SRP` 或 `PRJNA` 编号，再继续用 `metadata`、`convert` 或 `download` 处理。

## 常见工作流

### 从 GEO 项目下载对应 FASTQ

先把 GSE 转成 SRP：

```bash
sradb convert gse srp GSE56924
```

如果输出中得到 `SRP041298`，继续下载：

```bash
sradb download SRP041298 --out-dir ./sra -j 4
```

### 从论文编号找到数据集并获取元数据

```bash
sradb id 39528918 --json > ids.json
sradb metadata SRP174132 --detailed --format tsv > metadata.tsv
```

### 生成 run 清单

```bash
sradb convert srp srr SRP174132 > runs.tsv
```

如果只需要 run accession，可以用常见文本工具取第二列：

```bash
cut -f2 runs.tsv > run_accessions.txt
```

### 搜索并导出 RNA-seq 项目

```bash
sradb search \
  --organism "Homo sapiens" \
  --strategy RNA-Seq \
  --layout PAIRED \
  --platform ILLUMINA \
  --max 100 \
  --format tsv > human_rnaseq.tsv
```

## 调试和日志

所有子命令都支持全局 `-v`：

```bash
sradb -v metadata SRP174132
sradb -vv metadata SRP174132 --detailed
sradb -vvv search --query "ARID1A breast cancer" --max 5
```

日志级别含义：

- `-v`：info
- `-vv`：debug
- `-vvv`：trace

也可以使用 `RUST_LOG` 覆盖日志过滤：

```bash
RUST_LOG=sradb=debug,sradb_core=debug sradb metadata SRP174132
```

## 常见问题

### NCBI 请求变慢或失败

配置 `NCBI_EMAIL` 和 `NCBI_API_KEY`，并减少并发操作。NCBI API key 可以提高 eUtils 限速，但仍应避免短时间内发起过多大项目查询。

### `metadata --enrich` 报错

确认 `OPENAI_API_KEY` 已设置。如果使用非官方 OpenAI 兼容 endpoint，同时检查 `OPENAI_BASE_URL` 是否不带尾部 `/v1`。工具会自动请求：

```text
${OPENAI_BASE_URL}/v1/chat/completions
```

### `download` 没有下载到文件

`download` 依赖元数据中的 ENA FASTQ URL。某些项目可能没有公开 FASTQ，或者 ENA 暂未提供对应链接。可以先查看详细元数据：

```bash
sradb metadata <ACCESSION> --detailed --format tsv
```

重点检查 `ena_fastq_http_1` 和 `ena_fastq_http_2` 是否为空。

### 转换结果为空

部分 accession 之间没有公开映射，或者输入编号本身不属于声明的 `FROM` 类型。确认大小写和类型一致：

```bash
sradb convert srp srr SRP174132
```

不要写成：

```bash
sradb convert srp srr SRR8361601
```

因为 `SRR8361601` 会被解析为 `srr`，和 `FROM=srp` 不匹配。

## 开发者验证

在修改代码或文档示例后，可以运行：

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

离线测试主要依赖 `tests/data/` 下的 recorded fixtures；真实 API 测试通常通过特性开关手动运行。

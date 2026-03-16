//! csimulator CLI 入口
//!
//! 运行 V1-V8 验证示例并导出 JSON 轨迹。

use csimulator::benchmarks::{case_ids, get_case, list_cases};
use csimulator::lower::lower;
use csimulator::optimize::{collect_stats, optimize};
use csimulator::oracle::compare;
use csimulator::report::report_from_schedule;
use csimulator::schedule::{schedule_graph, SchedParams};
use csimulator::simulate::simulate;
use csimulator::trace::{report_summary, report_to_json, schedule_to_json, sim_trace_to_json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return;
    }

    if args.get(1).map(String::as_str) == Some("list") {
        print_case_list();
        return;
    }

    let case_id = args.get(1).map(String::as_str).unwrap_or("V1");
    let case = match get_case(case_id) {
        Some(case) => case,
        None => {
            eprintln!("Unknown case: {case_id}");
            eprintln!("Available cases: {}", case_ids().join(", "));
            eprintln!("Use 'cargo run -- list' to see descriptions.");
            std::process::exit(2);
        }
    };
    let output_dir = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts").join(case.id.to_lowercase()));

    println!("csimulator: BlockMat cycle-level simulator (phase 1)");
    println!("Case: {} - {}", case.id, case.title);
    println!("Desc: {}", case.description);
    println!("Out : {}\n", output_dir.display());

    let bm = case.block_mat;
    let input_blocks = case.input_blocks;

    // 优化
    let opt = optimize(&bm);
    let stats = collect_stats(&opt);
    println!("StructStats: {:?}\n", stats);

    // Lowering
    let graph = lower(&opt).expect("lower failed");
    println!(
        "SchedGraph: {} nodes, {} output nodes\n",
        graph.nodes.len(),
        graph.output_nodes.len()
    );

    // 调度
    let params = SchedParams::default_phase1();
    let schedule = schedule_graph(&graph, &params);
    println!(
        "Schedule: {} batches, total {} cycles\n",
        schedule.batches.len(),
        schedule.total_cycles
    );

    // 模拟
    let sim_result = simulate(&graph, &schedule, &params, &input_blocks);
    println!("SimResult output blocks:");
    for (i, block) in sim_result.output_vectors.iter().enumerate() {
        println!("  y[{}] = {:?}", i, block);
    }
    println!("  peak_input_buffer = {} blocks", sim_result.peak_input_buffer_blocks);
    println!("  peak_temp_buffer = {} blocks\n", sim_result.peak_temp_buffer);

    // Oracle 对照
    let oracle_result = compare(&opt, &input_blocks, &sim_result, 1e-6);
    println!(
        "Oracle: pass={}, max_abs_error={:.2e}, max_rel_error={:.2e}\n",
        oracle_result.pass, oracle_result.max_abs_error, oracle_result.max_rel_error
    );

    // 代价报告
    let report = report_from_schedule(&opt, &stats, &graph, &schedule, &sim_result, &params);
    println!("{}", report_summary(&report));

    export_artifacts(&output_dir, &schedule, &sim_result.per_cycle_trace, &report)
        .expect("failed to export JSON artifacts");
    println!("\nExported JSON artifacts to {}", output_dir.display());
}

fn export_artifacts(
    output_dir: &Path,
    schedule: &csimulator::schedule::Schedule,
    sim_trace: &[csimulator::simulate::CycleSnapshot],
    report: &csimulator::report::CostReport,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(output_dir)?;
    fs::write(output_dir.join("schedule.json"), schedule_to_json(schedule)?)?;
    fs::write(output_dir.join("sim_trace.json"), sim_trace_to_json(sim_trace)?)?;
    fs::write(output_dir.join("report.json"), report_to_json(report)?)?;
    Ok(())
}

fn print_case_list() {
    println!("Available benchmark cases:\n");
    for case in list_cases() {
        println!("  {} - {}", case.id, case.title);
        println!("      {}", case.description);
    }
}

fn print_usage() {
    println!("Usage:");
    println!("  cargo run -- [CASE_ID] [OUTPUT_DIR]");
    println!("  cargo run -- list");
    println!();
    println!("Examples:");
    println!("  cargo run -- V1");
    println!("  cargo run -- V6 artifacts/v6");
    println!("  cargo run -- list");
}

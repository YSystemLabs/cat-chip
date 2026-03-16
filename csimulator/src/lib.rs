pub mod benchmarks;
pub mod ir;
pub mod lower;
pub mod optimize;
pub mod oracle;
pub mod report;
pub mod schedule;
pub mod semantics;
pub mod simulate;
pub mod trace;

#[cfg(test)]
mod spec_tests {
	use crate::benchmarks::get_case;
	use crate::ir::{BlockMat, D};
	use crate::lower::lower;
	use crate::optimize::{collect_stats, optimize};
	use crate::oracle::compare;
	use crate::report::report_from_schedule;
	use crate::schedule::{schedule_graph, SchedParams};
	use crate::simulate::simulate;

	fn run_pipeline(
		bm: BlockMat,
		input_blocks: Vec<[f32; D]>,
	) -> (crate::optimize::StructStats, crate::schedule::Schedule, crate::simulate::SimResult, crate::report::CostReport) {
		let opt = optimize(&bm);
		let stats = collect_stats(&opt);
		let graph = lower(&opt).unwrap();
		let params = SchedParams::default_phase1();
		let schedule = schedule_graph(&graph, &params);
		let sim = simulate(&graph, &schedule, &params, &input_blocks);
		let oracle = compare(&opt, &input_blocks, &sim, 1e-6);
		assert!(oracle.pass, "oracle mismatch: abs={}, rel={}", oracle.max_abs_error, oracle.max_rel_error);
		let report = report_from_schedule(&opt, &stats, &graph, &schedule, &sim, &params);
		(stats, schedule, sim, report)
	}

	#[test]
	fn test_v1_dense_end_to_end() {
		let case = get_case("V1").unwrap();
		let (_, _, _, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert!(report.analytic_total_eff > 0);
		assert!(report.sched_total_cycles > 0);
	}

	#[test]
	fn test_v2_zero_block_elimination() {
		let case = get_case("V2").unwrap();
		let (stats, _, _, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(stats.n_nz, 5);
		assert_eq!(report.dead_cols, 0);
	}

	#[test]
	fn test_v3_identity_specialization() {
		let case = get_case("V3").unwrap();
		let (stats, _, _, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(stats.n_gen, 3);
		assert_eq!(stats.single_rows, 0);
		assert!(report.analytic_exec_cycles > 0);
	}

	#[test]
	fn test_v4_block_diagonal_single_rows() {
		let case = get_case("V4").unwrap();
		let (stats, schedule, _, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(stats.single_rows, 3);
		assert_eq!(report.reduce_rows, 0);
		assert_eq!(schedule.total_cycles, report.sched_total_cycles);
	}

	#[test]
	fn test_v5_permutation_is_zero_cost_structure() {
		let case = get_case("V5").unwrap();
		let (stats, schedule, _, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert!(stats.is_permutation);
		assert_eq!(report.analytic_exec_cycles, 0);
		assert_eq!(report.sched_total_cycles, 0);
		assert_eq!(schedule.total_cycles, 0);
	}

	#[test]
	fn test_v6_large_mixed_end_to_end() {
		let case = get_case("V6").unwrap();
		let (stats, _, sim, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(sim.output_vectors.len(), 3);
		assert!(stats.n_unique <= stats.n_gen);
		assert!(report.peak_temp_buffer_blocks >= 1);
		assert!(report.flat_total >= report.analytic_total_eff);
	}

	#[test]
	fn test_v7_biproduct_sum_end_to_end() {
		let case = get_case("V7").unwrap();
		let (stats, _, sim, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(sim.output_vectors.len(), 2);
		assert_eq!(stats.n_nz, 2);
		assert_eq!(report.reduce_rows, 0);
		assert!(report.sched_total_cycles > 0);
	}

	#[test]
	fn test_v8_hundred_dim_banded_end_to_end() {
		let case = get_case("V8").unwrap();
		let (stats, schedule, sim, report) = run_pipeline(case.block_mat, case.input_blocks);
		assert_eq!(sim.output_vectors.len(), 64);
		assert_eq!(stats.n_nz, 132);
		assert_eq!(stats.dead_cols, 22);
		assert_eq!(stats.dead_rows, 4);
		assert_eq!(stats.single_rows, 20);
		assert_eq!(stats.n_unique, 6);
		assert_eq!(stats.n_scale_u, 3);
		assert_eq!(stats.n_scale, 48);
		assert_eq!(report.reduce_rows, 40);
		assert_eq!(report.bcast_cols, 36);
		assert_eq!(report.peak_input_buffer_blocks, 42);
		assert!(schedule.total_cycles > 0);
	}

	#[test]
	fn test_r5_dead_col_reduces_peak_input_buffer() {
		let live: BlockMat = BlockMat {
			rows: 2,
			cols: 3,
			grid: vec![
				vec![
					crate::ir::KernelRef::General([[1.0, 0.0], [0.0, 1.0]]),
					crate::ir::KernelRef::General([[2.0, 0.0], [0.0, 2.0]]),
					crate::ir::KernelRef::General([[1.0, 1.0], [0.0, 1.0]]),
				],
				vec![
					crate::ir::KernelRef::General([[1.0, 0.0], [0.0, 1.0]]),
					crate::ir::KernelRef::General([[1.0, 1.0], [0.0, 1.0]]),
					crate::ir::KernelRef::General([[2.0, 0.0], [0.0, 2.0]]),
				],
			],
		};
		let dead_col: BlockMat = BlockMat {
			rows: 2,
			cols: 3,
			grid: vec![
				vec![
					crate::ir::KernelRef::General([[1.0, 0.0], [0.0, 1.0]]),
					crate::ir::KernelRef::General([[2.0, 0.0], [0.0, 2.0]]),
					crate::ir::KernelRef::Zero,
				],
				vec![
					crate::ir::KernelRef::General([[1.0, 0.0], [0.0, 1.0]]),
					crate::ir::KernelRef::General([[1.0, 1.0], [0.0, 1.0]]),
					crate::ir::KernelRef::Zero,
				],
			],
		};

		let common_input = vec![[1.0, 0.0], [0.0, 1.0], [2.0, 3.0]];
		let (_, _, _, live_report) = run_pipeline(live, common_input.clone());
		let (dead_stats, _, dead_sim, dead_report) = run_pipeline(dead_col, common_input);

		assert_eq!(dead_stats.dead_cols, 1);
		assert!(dead_report.peak_input_buffer_blocks < live_report.peak_input_buffer_blocks);
		assert_eq!(dead_report.peak_input_buffer_blocks, 2);
		assert_eq!(
			dead_sim.per_cycle_trace.last().map(|snapshot| snapshot.input_buffer_occupancy),
			Some(0)
		);
	}
}

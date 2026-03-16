#!/usr/bin/env python3

import argparse
import json
from html import escape
from pathlib import Path


PALETTE = {
    "bg": "#f6f2e8",
    "ink": "#1f252b",
    "muted": "#61707d",
    "grid": "#d8cfbf",
    "load": "#d77a61",
    "exec": "#3b6ea8",
    "writeback": "#4f8a5b",
    "buffer_input": "#1f7a8c",
    "buffer_temp_end": "#d95d39",
    "buffer_temp_peak": "#7c5cff",
    "buffer_output": "#3d405b",
    "card": "#fffdf8",
    "accent": "#c8553d",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render schedule and buffer-occupancy visualizations from csimulator JSON artifacts."
    )
    parser.add_argument(
        "artifact_dir",
        type=Path,
        help="Directory containing schedule.json, sim_trace.json, and optionally report.json",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Directory for generated SVG/HTML files (default: <artifact_dir>/viz)",
    )
    parser.add_argument(
        "--title",
        type=str,
        default=None,
        help="Optional page title override",
    )
    return parser.parse_args()


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def load_artifacts(artifact_dir: Path):
    schedule = read_json(artifact_dir / "schedule.json")
    sim_trace = read_json(artifact_dir / "sim_trace.json")
    report_path = artifact_dir / "report.json"
    report = read_json(report_path) if report_path.exists() else None
    return schedule, sim_trace, report


def classify_node(timed_node: dict, total_cycles: int) -> str:
    duration = timed_node["finish_cycle"] - timed_node["start_cycle"]
    if timed_node["start_cycle"] == 0 and duration <= 1:
        return "load"
    if timed_node["finish_cycle"] >= total_cycles:
        return "writeback"
    return "exec"


def render_schedule_svg(schedule: dict, title: str) -> str:
    timed_nodes = schedule.get("timed_nodes", [])
    total_cycles = max(schedule.get("total_cycles", 0), 1)
    left = 90
    top = 50
    row_height = 32
    bar_height = 18
    cycle_width = 84
    width = left + total_cycles * cycle_width + 50
    height = top + len(timed_nodes) * row_height + 50

    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        f'<rect width="{width}" height="{height}" fill="{PALETTE["bg"]}"/>',
        f'<text x="{left}" y="28" font-size="20" font-family="Georgia, serif" fill="{PALETTE["ink"]}">{escape(title)}</text>',
    ]

    for cycle in range(total_cycles + 1):
        x = left + cycle * cycle_width
        lines.append(
            f'<line x1="{x}" y1="{top - 10}" x2="{x}" y2="{height - 24}" stroke="{PALETTE["grid"]}" stroke-width="1"/>'
        )
        if cycle < total_cycles:
            lines.append(
                f'<text x="{x + cycle_width / 2}" y="{top - 16}" text-anchor="middle" font-size="12" fill="{PALETTE["muted"]}">c{cycle}</text>'
            )

    for idx, node in enumerate(timed_nodes):
        row_y = top + idx * row_height
        node_id = node["id"]
        start = node["start_cycle"]
        finish = node["finish_cycle"]
        duration = max(finish - start, 0.15)
        category = classify_node(node, schedule.get("total_cycles", 0))
        color = PALETTE[category]
        x = left + start * cycle_width + 4
        width_bar = max(duration * cycle_width - 8, 12)
        lines.append(
            f'<text x="20" y="{row_y + 14}" font-size="12" fill="{PALETTE["ink"]}">node {node_id}</text>'
        )
        lines.append(
            f'<rect x="{x}" y="{row_y}" rx="6" ry="6" width="{width_bar}" height="{bar_height}" fill="{color}" opacity="0.92"/>'
        )
        lines.append(
            f'<text x="{x + width_bar / 2}" y="{row_y + 13}" text-anchor="middle" font-size="11" fill="#ffffff">{category} {start}->{finish}</text>'
        )

    legend_y = height - 16
    legend = [("load", "load/input"), ("exec", "core/direct/reduce"), ("writeback", "writeback")]
    legend_x = left
    for key, label in legend:
        lines.append(
            f'<rect x="{legend_x}" y="{legend_y - 10}" width="14" height="14" fill="{PALETTE[key]}" rx="3" ry="3"/>'
        )
        lines.append(
            f'<text x="{legend_x + 20}" y="{legend_y + 1}" font-size="12" fill="{PALETTE["ink"]}">{label}</text>'
        )
        legend_x += 140

    lines.append("</svg>")
    return "\n".join(lines)


def y_scale(value: float, max_value: float, top: int, height: int) -> float:
    if max_value <= 0:
        return top + height
    return top + height - (value / max_value) * height


def polyline_points(series, max_value: float, left: int, top: int, height: int, cycle_width: int) -> str:
    return " ".join(
        f'{left + point["cycle"] * cycle_width},{y_scale(point["value"], max_value, top, height):.2f}'
        for point in series
    )


def render_buffer_svg(sim_trace: list[dict], title: str) -> str:
    if not sim_trace:
        sim_trace = [{"cycle": 0, "input_buffer_occupancy": 0, "end_of_cycle_occupancy": 0, "instant_peak_occupancy": 0, "output_buffer_occupancy": 0}]

    left = 70
    top = 40
    chart_height = 260
    cycle_width = 88
    last_cycle = sim_trace[-1]["cycle"]
    width = left + (last_cycle + 1) * cycle_width + 30
    height = top + chart_height + 65
    max_value = max(
        max(snapshot["input_buffer_occupancy"] for snapshot in sim_trace),
        max(snapshot["end_of_cycle_occupancy"] for snapshot in sim_trace),
        max(snapshot["instant_peak_occupancy"] for snapshot in sim_trace),
        max(snapshot["output_buffer_occupancy"] for snapshot in sim_trace),
        1,
    )

    series = {
        "input": [
            {"cycle": snap["cycle"], "value": snap["input_buffer_occupancy"]} for snap in sim_trace
        ],
        "temp_end": [
            {"cycle": snap["cycle"], "value": snap["end_of_cycle_occupancy"]} for snap in sim_trace
        ],
        "temp_peak": [
            {"cycle": snap["cycle"], "value": snap["instant_peak_occupancy"]} for snap in sim_trace
        ],
        "output": [
            {"cycle": snap["cycle"], "value": snap["output_buffer_occupancy"]} for snap in sim_trace
        ],
    }

    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        f'<rect width="{width}" height="{height}" fill="{PALETTE["bg"]}"/>',
        f'<text x="{left}" y="24" font-size="20" font-family="Georgia, serif" fill="{PALETTE["ink"]}">{escape(title)}</text>',
    ]

    for value in range(max_value + 1):
        y = y_scale(value, max_value, top, chart_height)
        lines.append(
            f'<line x1="{left}" y1="{y}" x2="{width - 20}" y2="{y}" stroke="{PALETTE["grid"]}" stroke-width="1"/>'
        )
        lines.append(
            f'<text x="{left - 12}" y="{y + 4}" text-anchor="end" font-size="12" fill="{PALETTE["muted"]}">{value}</text>'
        )

    for snapshot in sim_trace:
        x = left + snapshot["cycle"] * cycle_width
        lines.append(
            f'<line x1="{x}" y1="{top}" x2="{x}" y2="{top + chart_height}" stroke="{PALETTE["grid"]}" stroke-width="1"/>'
        )
        lines.append(
            f'<text x="{x}" y="{top + chart_height + 20}" text-anchor="middle" font-size="12" fill="{PALETTE["muted"]}">c{snapshot["cycle"]}</text>'
        )

    color_map = {
        "input": PALETTE["buffer_input"],
        "temp_end": PALETTE["buffer_temp_end"],
        "temp_peak": PALETTE["buffer_temp_peak"],
        "output": PALETTE["buffer_output"],
    }
    label_map = {
        "input": "input buffer",
        "temp_end": "temp end-of-cycle",
        "temp_peak": "temp instant peak",
        "output": "output buffer",
    }

    for key in ["input", "temp_end", "temp_peak", "output"]:
        points = polyline_points(series[key], max_value, left, top, chart_height, cycle_width)
        lines.append(
            f'<polyline fill="none" stroke="{color_map[key]}" stroke-width="3" points="{points}"/>'
        )
        for point in series[key]:
            cx = left + point["cycle"] * cycle_width
            cy = y_scale(point["value"], max_value, top, chart_height)
            lines.append(f'<circle cx="{cx}" cy="{cy}" r="4" fill="{color_map[key]}"/>')

    legend_y = top + chart_height + 44
    legend_x = left
    for key in ["input", "temp_end", "temp_peak", "output"]:
        lines.append(
            f'<line x1="{legend_x}" y1="{legend_y}" x2="{legend_x + 16}" y2="{legend_y}" stroke="{color_map[key]}" stroke-width="4"/>'
        )
        lines.append(
            f'<text x="{legend_x + 22}" y="{legend_y + 4}" font-size="12" fill="{PALETTE["ink"]}">{label_map[key]}</text>'
        )
        legend_x += 165

    lines.append("</svg>")
    return "\n".join(lines)


def metric_cards(report: dict | None) -> str:
    if not report:
        return ""
    entries = [
        ("total cycles", report.get("sched_total_cycles", 0)),
        ("peak input", report.get("peak_input_buffer_blocks", 0)),
        ("peak temp", report.get("peak_temp_buffer_blocks", 0)),
        ("peak output", report.get("peak_output_buffer_blocks", 0)),
        ("flat baseline", report.get("flat_total", 0)),
        ("cost ratio", f'{report.get("cost_ratio", 0.0):.4f}'),
    ]
    cards = []
    for label, value in entries:
        cards.append(
            f'<div style="background:{PALETTE["card"]};border:1px solid {PALETTE["grid"]};border-radius:14px;padding:14px 16px;min-width:140px">'
            f'<div style="font-size:12px;color:{PALETTE["muted"]};text-transform:uppercase;letter-spacing:0.08em">{escape(str(label))}</div>'
            f'<div style="margin-top:6px;font-size:24px;color:{PALETTE["ink"]};font-weight:700">{escape(str(value))}</div>'
            f'</div>'
        )
    return "\n".join(cards)


def render_html(title: str, schedule_svg_name: str, buffer_svg_name: str, report: dict | None) -> str:
    summary = ""
    if report:
        summary = (
            f'<div style="display:flex;flex-wrap:wrap;gap:12px;margin:18px 0 28px 0">{metric_cards(report)}</div>'
        )

    return f"""<!DOCTYPE html>
<html lang=\"zh-CN\">
<head>
  <meta charset=\"utf-8\" />
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
  <title>{escape(title)}</title>
  <style>
    body {{
      margin: 0;
      font-family: 'Noto Serif SC', 'Source Han Serif SC', Georgia, serif;
      background: {PALETTE['bg']};
      color: {PALETTE['ink']};
    }}
    main {{
      max-width: 1180px;
      margin: 0 auto;
      padding: 28px 24px 40px;
    }}
    h1 {{
      margin: 0;
      font-size: 34px;
      line-height: 1.1;
    }}
    p {{
      color: {PALETTE['muted']};
      max-width: 860px;
      line-height: 1.6;
    }}
    section {{
      margin-top: 26px;
      background: rgba(255,255,255,0.55);
      border: 1px solid {PALETTE['grid']};
      border-radius: 20px;
      padding: 18px;
      overflow-x: auto;
    }}
    img {{
      width: 100%;
      height: auto;
      display: block;
    }}
  </style>
</head>
<body>
  <main>
    <h1>{escape(title)}</h1>
    <p>该页面由 visualize_artifacts.py 从 csimulator 导出的 JSON artifact 自动生成，包含调度甘特图、缓冲占用曲线和关键成本指标。</p>
    {summary}
    <section>
      <img src=\"{escape(schedule_svg_name)}\" alt=\"schedule gantt\" />
    </section>
    <section>
      <img src=\"{escape(buffer_svg_name)}\" alt=\"buffer occupancy\" />
    </section>
  </main>
</body>
</html>
"""


def main() -> None:
    args = parse_args()
    artifact_dir = args.artifact_dir.resolve()
    output_dir = (args.output_dir or artifact_dir / "viz").resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    schedule, sim_trace, report = load_artifacts(artifact_dir)
    title = args.title or f"Artifact Visualization: {artifact_dir.name}"

    schedule_svg = render_schedule_svg(schedule, f"{title} · Schedule")
    buffer_svg = render_buffer_svg(sim_trace, f"{title} · Buffer Occupancy")
    html = render_html(title, "schedule_gantt.svg", "buffer_occupancy.svg", report)

    (output_dir / "schedule_gantt.svg").write_text(schedule_svg, encoding="utf-8")
    (output_dir / "buffer_occupancy.svg").write_text(buffer_svg, encoding="utf-8")
    (output_dir / "index.html").write_text(html, encoding="utf-8")

    print(f"Wrote {output_dir / 'schedule_gantt.svg'}")
    print(f"Wrote {output_dir / 'buffer_occupancy.svg'}")
    print(f"Wrote {output_dir / 'index.html'}")


if __name__ == "__main__":
    main()
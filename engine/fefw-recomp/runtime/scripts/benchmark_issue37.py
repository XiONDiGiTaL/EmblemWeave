#!/usr/bin/env python3
"""
Issue #37 Phase 1: Renderer Profiling Benchmark Harness

Runs 7 benchmark scenarios and collects CSV profiles for comparison.

Scenarios:
1. Cold Startup - First launch, empty caches
2. First Gameplay Entry - Initial level load
3. Shader-Heavy Scenes - Complex materials, many variants
4. Repeated Visits - Return to previous areas
5. Combat Scenes - High draw call count, effects
6. Scene Transitions - Load/unload resources
7. Warm Startup - Subsequent launches with caches

Usage:
    python benchmark_issue37.py --launcher <path_to_mpr_launcher.exe> --output-dir <dir> [--scenarios 1,2,3]
"""

import argparse
import subprocess
import time
import os
import sys
import json
import csv
from pathlib import Path
from datetime import datetime
from typing import List, Dict, Optional

SCENARIOS = {
    1: {
        "name": "cold_startup",
        "description": "First launch, empty shader/pipeline caches",
        "args": ["--benchmark-mode", "cold_startup"],
        "duration_sec": 30,
        "key_metrics": ["shader_translation", "pipeline_creation", "texture_upload", "memory_alloc"]
    },
    2: {
        "name": "first_gameplay_entry",
        "description": "Initial level load after startup",
        "args": ["--benchmark-mode", "first_gameplay"],
        "duration_sec": 60,
        "key_metrics": ["pipeline_creation", "texture_creation", "cache_misses"]
    },
    3: {
        "name": "shader_heavy_scenes",
        "description": "Complex materials, many shader variants",
        "args": ["--benchmark-mode", "shader_heavy"],
        "duration_sec": 60,
        "key_metrics": ["shader_translation", "pipeline_creation", "cache_hit_rate"]
    },
    4: {
        "name": "repeated_visits",
        "description": "Return to previously visited areas",
        "args": ["--benchmark-mode", "repeated_visits"],
        "duration_sec": 60,
        "key_metrics": ["cache_hits", "prewarm_effectiveness", "memory_stability"]
    },
    4: {
        "name": "combat_scenes",
        "description": "High draw call count, particle effects",
        "args": ["--benchmark-mode", "combat"],
        "duration_sec": 60,
        "key_metrics": ["submission_time", "gpu_sync", "render_thread_cpu"]
    },
    5: {
        "name": "scene_transitions",
        "description": "Rapid load/unload of resources",
        "args": ["--benchmark-mode", "scene_transitions"],
        "duration_sec": 60,
        "key_metrics": ["texture_destruction_creation", "upload", "memory_pressure"]
    },
    6: {
        "name": "warm_startup",
        "description": "Subsequent launch with populated caches",
        "args": ["--benchmark-mode", "warm_startup"],
        "duration_sec": 30,
        "key_metrics": ["cache_hit_rates", "prewarm_queue", "startup_time"]
    }
}

# Note: Scenario 4 is duplicated above, fixing:
SCENARIOS = {
    1: {
        "name": "cold_startup",
        "description": "First launch, empty shader/pipeline caches",
        "args": ["--benchmark-mode", "cold_startup"],
        "duration_sec": 30,
        "key_metrics": ["shader_translation", "pipeline_creation", "texture_upload", "memory_alloc"]
    },
    2: {
        "name": "first_gameplay_entry",
        "description": "Initial level load after startup",
        "args": ["--benchmark-mode", "first_gameplay"],
        "duration_sec": 60,
        "key_metrics": ["pipeline_creation", "texture_creation", "cache_misses"]
    },
    3: {
        "name": "shader_heavy_scenes",
        "description": "Complex materials, many shader variants",
        "args": ["--benchmark-mode", "shader_heavy"],
        "duration_sec": 60,
        "key_metrics": ["shader_translation", "pipeline_creation", "cache_hit_rate"]
    },
    4: {
        "name": "repeated_visits",
        "description": "Return to previously visited areas",
        "args": ["--benchmark-mode", "repeated_visits"],
        "duration_sec": 60,
        "key_metrics": ["cache_hits", "prewarm_effectiveness", "memory_stability"]
    },
    5: {
        "name": "combat_scenes",
        "description": "High draw call count, particle effects",
        "args": ["--benchmark-mode", "combat"],
        "duration_sec": 60,
        "key_metrics": ["submission_time", "gpu_sync", "render_thread_cpu"]
    },
    6: {
        "name": "scene_transitions",
        "description": "Rapid load/unload of resources",
        "args": ["--benchmark-mode", "scene_transitions"],
        "duration_sec": 60,
        "key_metrics": ["texture_destruction_creation", "upload", "memory_pressure"]
    },
    7: {
        "name": "warm_startup",
        "description": "Subsequent launch with populated caches",
        "args": ["--benchmark-mode", "warm_startup"],
        "duration_sec": 30,
        "key_metrics": ["cache_hit_rates", "prewarm_queue", "startup_time"]
    }
}


class BenchmarkRunner:
    def __init__(self, launcher_path: str, output_dir: str, enable_profiling: bool = True):
        self.launcher_path = Path(launcher_path).resolve()
        self.output_dir = Path(output_dir).resolve()
        self.enable_profiling = enable_profiling
        self.results = []

        if not self.launcher_path.exists():
            raise FileNotFoundError(f"Launcher not found: {self.launcher_path}")

        self.output_dir.mkdir(parents=True, exist_ok=True)

    def run_scenario(self, scenario_id: int, extra_env: Dict[str, str] = None) -> Dict:
        """Run a single benchmark scenario."""
        scenario = SCENARIOS[scenario_id]
        print(f"\n{'='*60}")
        print(f"Scenario {scenario_id}: {scenario['name']}")
        print(f"Description: {scenario['description']}")
        print(f"Duration: {scenario['duration_sec']}s")
        print(f"{'='*60}")

        # Prepare environment
        env = os.environ.copy()
        if self.enable_profiling:
            env["NXRT_PROF_RENDERER_PROFILING"] = "1"
            csv_path = self.output_dir / f"profile_{scenario['name']}.csv"
            env["NXRT_PROF_RENDERER_CSV"] = str(csv_path)
            print(f"Profiling enabled -> {csv_path}")

        if extra_env:
            env.update(extra_env)

        # Build command
        cmd = [str(self.launcher_path)] + scenario["args"]
        print(f"Command: {' '.join(cmd)}")

        # Run benchmark
        start_time = time.time()
        try:
            # Run with timeout (duration + buffer)
            timeout = scenario["duration_sec"] + 30
            result = subprocess.run(
                cmd,
                env=env,
                timeout=timeout,
                capture_output=True,
                text=True
            )
            elapsed = time.time() - start_time

            run_result = {
                "scenario_id": scenario_id,
                "scenario_name": scenario["name"],
                "success": result.returncode == 0,
                "elapsed_sec": elapsed,
                "returncode": result.returncode,
                "stdout": result.stdout[-5000:] if result.stdout else "",
                "stderr": result.stderr[-5000:] if result.stderr else "",
                "csv_path": str(csv_path) if self.enable_profiling else None
            }

            if result.returncode == 0:
                print(f"✓ Completed in {elapsed:.1f}s")
            else:
                print(f"✗ Failed (code {result.returncode}) in {elapsed:.1f}s")
                if result.stderr:
                    print(f"  stderr: {result.stderr[:500]}")

            return run_result

        except subprocess.TimeoutExpired:
            elapsed = time.time() - start_time
            print(f"✗ Timeout after {elapsed:.1f}s")
            return {
                "scenario_id": scenario_id,
                "scenario_name": scenario["name"],
                "success": False,
                "elapsed_sec": elapsed,
                "returncode": -1,
                "error": "timeout",
                "csv_path": str(csv_path) if self.enable_profiling else None
            }
        except Exception as e:
            elapsed = time.time() - start_time
            print(f"✗ Error: {e}")
            return {
                "scenario_id": scenario_id,
                "scenario_name": scenario["name"],
                "success": False,
                "elapsed_sec": elapsed,
                "returncode": -1,
                "error": str(e),
                "csv_path": str(csv_path) if self.enable_profiling else None
            }

    def run_all(self, scenario_ids: List[int] = None) -> List[Dict]:
        """Run all or selected scenarios."""
        if scenario_ids is None:
            scenario_ids = list(SCENARIOS.keys())

        print(f"Starting Issue #37 benchmark suite")
        print(f"Launcher: {self.launcher_path}")
        print(f"Output dir: {self.output_dir}")
        print(f"Scenarios: {scenario_ids}")
        print(f"Profiling: {'enabled' if self.enable_profiling else 'disabled'}")

        for sid in scenario_ids:
            if sid not in SCENARIOS:
                print(f"Warning: Unknown scenario {sid}, skipping")
                continue

            result = self.run_scenario(sid)
            self.results.append(result)

            # Brief pause between scenarios
            if sid != scenario_ids[-1]:
                print("Waiting 5s before next scenario...")
                time.sleep(5)

        return self.results

    def generate_report(self) -> str:
        """Generate a summary report."""
        report_path = self.output_dir / f"benchmark_report_{datetime.now().strftime('%Y%m%d_%H%M%S')}.json"

        report = {
            "timestamp": datetime.now().isoformat(),
            "launcher": str(self.launcher_path),
            "profiling_enabled": self.enable_profiling,
            "scenarios_run": len(self.results),
            "results": self.results
        }

        with open(report_path, 'w') as f:
            json.dump(report, f, indent=2)

        # Also generate CSV summary
        csv_report_path = self.output_dir / f"benchmark_summary_{datetime.now().strftime('%Y%m%d_%H%M%S')}.csv"
        with open(csv_report_path, 'w', newline='') as f:
            writer = csv.writer(f)
            writer.writerow(["Scenario", "Name", "Success", "Elapsed (s)", "Return Code", "CSV Profile"])
            for r in self.results:
                writer.writerow([
                    r["scenario_id"],
                    r["scenario_name"],
                    "PASS" if r["success"] else "FAIL",
                    f"{r['elapsed_sec']:.1f}",
                    r["returncode"],
                    r.get("csv_path", "N/A")
                ])

        print(f"\n{'='*60}")
        print("BENCHMARK SUMMARY")
        print(f"{'='*60}")
        for r in self.results:
            status = "PASS" if r["success"] else "FAIL"
            print(f"  {r['scenario_id']}: {r['scenario_name']:25s} {status:4s} {r['elapsed_sec']:6.1f}s")

        passed = sum(1 for r in self.results if r["success"])
        print(f"\nTotal: {passed}/{len(self.results)} passed")
        print(f"Report: {report_path}")
        print(f"Summary: {csv_report_path}")

        return str(report_path)

    def analyze_profiles(self) -> Dict:
        """Analyze generated CSV profiles for key metrics."""
        if not self.enable_profiling:
            return {}

        analysis = {}
        for r in self.results:
            if not r.get("csv_path") or not Path(r["csv_path"]).exists():
                continue

            scenario_name = r["scenario_name"]
            try:
                with open(r["csv_path"], 'r') as f:
                    reader = csv.DictReader(f)
                    rows = list(reader)

                if not rows:
                    continue

                # Get last row (final snapshot)
                last = rows[-1]

                # Extract key metrics (adjust column names to match actual CSV)
                metrics = {}
                for key in last.keys():
                    if any(kw in key.lower() for kw in ['p50', 'p95', 'p99', 'max', 'avg', 'total', 'count', 'hits', 'misses', 'bytes', 'ms']):
                        try:
                            metrics[key] = float(last[key])
                        except ValueError:
                            metrics[key] = last[key]

                analysis[scenario_name] = {
                    "frame_count": len(rows),
                    "final_metrics": metrics
                }

            except Exception as e:
                print(f"Warning: Failed to analyze {r['csv_path']}: {e}")

        return analysis


def main():
    parser = argparse.ArgumentParser(description="Issue #37 Renderer Profiling Benchmark")
    parser.add_argument("--launcher", help="Path to mpr_launcher.exe")
    parser.add_argument("--output-dir", default="./benchmark_results", help="Output directory for results")
    parser.add_argument("--scenarios", help="Comma-separated scenario IDs (1-7), default: all")
    parser.add_argument("--no-profiling", action="store_true", help="Disable renderer profiling")
    parser.add_argument("--list", action="store_true", help="List scenarios and exit")

    args = parser.parse_args()

    if args.list:
        print("Available scenarios:")
        for sid, s in SCENARIOS.items():
            print(f"  {sid}: {s['name']} - {s['description']}")
        return 0

    if not args.launcher:
        parser.error("--launcher is required unless --list is used")

    scenario_ids = None
    if args.scenarios:
        try:
            scenario_ids = [int(x.strip()) for x in args.scenarios.split(",")]
        except ValueError:
            print("Error: --scenarios must be comma-separated integers")
            return 1

    runner = BenchmarkRunner(
        launcher_path=args.launcher,
        output_dir=args.output_dir,
        enable_profiling=not args.no_profiling
    )

    runner.run_all(scenario_ids)
    runner.generate_report()

    if not args.no_profiling:
        print("\nAnalyzing profiles...")
        analysis = runner.analyze_profiles()
        for name, data in analysis.items():
            print(f"\n{name}:")
            for key, val in data["final_metrics"].items():
                print(f"  {key}: {val}")

    # Exit with error if any scenario failed
    failed = sum(1 for r in runner.results if not r["success"])
    return 1 if failed > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
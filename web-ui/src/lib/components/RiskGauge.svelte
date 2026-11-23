<script lang="ts">
	import type { RiskSummary } from '$lib/api/types';
	import { decimalDirectionIcon, formatDecimalString } from '$lib/format/decimal';

	interface Props {
		risk: RiskSummary;
	}

	let { risk }: Props = $props();

	function clampPct(numerator: string, denominator: string): number {
		const n = Math.abs(Number(numerator));
		const d = Math.abs(Number(denominator));
		if (!Number.isFinite(n) || !Number.isFinite(d) || d <= 0) return 0;
		return Math.min(100, Math.round((n / d) * 100));
	}

	const heatBar = $derived(clampPct(risk.heat_pct, risk.heat_max_pct));
	const ddBar = $derived(clampPct(risk.drawdown_pct, risk.limits.kill_pct));
	const posBar = $derived(
		risk.max_positions > 0
			? Math.min(100, Math.round((risk.open_positions / risk.max_positions) * 100))
			: 0
	);
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
	<div class="mb-3 flex items-center justify-between">
		<h2 class="text-xs font-bold tracking-wider text-slate-400 uppercase">Risk</h2>
		<span
			class={`rounded px-2 py-0.5 text-xs font-semibold ${
				risk.btc_guard === 'SECURE'
					? 'bg-emerald-500/15 text-emerald-300'
					: 'bg-red-500/20 text-red-300'
			}`}
		>
			BTC Guard: {risk.btc_guard}
		</span>
	</div>

	<div class="space-y-3 text-xs">
		<div>
			<div class="mb-1 flex justify-between font-mono">
				<span class="text-slate-400">Heat</span>
				<span>
					{formatDecimalString(risk.heat_pct)}% / {formatDecimalString(risk.heat_max_pct)}%
				</span>
			</div>
			<div class="h-2 overflow-hidden rounded bg-slate-800">
				<div
					class={`h-full ${heatBar >= 80 ? 'bg-red-500' : heatBar >= 50 ? 'bg-amber-400' : 'bg-sky-400'}`}
					style={`width: ${heatBar}%`}
				></div>
			</div>
		</div>

		<div>
			<div class="mb-1 flex justify-between font-mono">
				<span class="text-slate-400">Day PnL (halt {formatDecimalString(risk.limits.daily_halt_pct)}%)</span>
				<span>
					{decimalDirectionIcon(risk.day_pnl_pct)}
					{formatDecimalString(risk.day_pnl_pct, { signed: true, suffix: '%' })}
				</span>
			</div>
		</div>

		<div>
			<div class="mb-1 flex justify-between font-mono">
				<span class="text-slate-400">Drawdown (kill {formatDecimalString(risk.limits.kill_pct)}%)</span>
				<span>{formatDecimalString(risk.drawdown_pct, { suffix: '%' })}</span>
			</div>
			<div class="h-2 overflow-hidden rounded bg-slate-800">
				<div
					class={`h-full ${ddBar >= 70 ? 'bg-red-500' : 'bg-amber-500'}`}
					style={`width: ${ddBar}%`}
				></div>
			</div>
		</div>

		<div>
			<div class="mb-1 flex justify-between font-mono">
				<span class="text-slate-400">Open Positions</span>
				<span>{risk.open_positions} / {risk.max_positions}</span>
			</div>
			<div class="h-2 overflow-hidden rounded bg-slate-800">
				<div class="h-full bg-indigo-400" style={`width: ${posBar}%`}></div>
			</div>
		</div>
	</div>
</section>

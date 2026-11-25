<script lang="ts">
	import type { RiskSummary } from '$lib/api/types';
	import Panel from '$lib/components/ui/Panel.svelte';
	import StatusPill from '$lib/components/ui/StatusPill.svelte';
	import { decimalDirectionIcon, decimalSign, formatDecimalString } from '$lib/format/decimal';

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
	const daySign = $derived(decimalSign(risk.day_pnl_pct));
	const btcSecure = $derived(risk.btc_guard === 'SECURE');
</script>

<Panel title="Risk & Guards" subtitle="Account Aggregate">
	{#snippet actions()}
		<StatusPill
			label={`BTC Guard: ${risk.btc_guard}`}
			icon={btcSecure ? '●' : '⚠'}
			tone={btcSecure ? 'positive' : 'danger'}
		/>
	{/snippet}

	<div class="space-y-3.5 text-xs">
		<div>
			<div class="mb-1.5 flex justify-between font-mono tabular-nums">
				<span class="text-ink-muted">Portfolio Heat</span>
				<span class="text-ink-strong">
					{formatDecimalString(risk.heat_pct)}% / {formatDecimalString(risk.heat_max_pct)}%
				</span>
			</div>
			<div class="h-1.5 overflow-hidden rounded-full bg-surface-inset">
				<div
					class={`h-full transition-[width] duration-200 ${
						heatBar >= 80 ? 'bg-danger-solid' : heatBar >= 50 ? 'bg-warning' : 'bg-telemetry'
					}`}
					style={`width: ${heatBar}%`}
				></div>
			</div>
		</div>

		<div class="flex items-center justify-between rounded border border-hairline-subtle bg-surface-inset px-3 py-2 font-mono tabular-nums">
			<span class="text-ink-muted">
				Day PnL <span class="text-ink-dim">(halt {formatDecimalString(risk.limits.daily_halt_pct)}%)</span>
			</span>
			<span
				class={`font-semibold ${
					daySign > 0 ? 'text-positive' : daySign < 0 ? 'text-danger' : 'text-ink-strong'
				}`}
			>
				{decimalDirectionIcon(risk.day_pnl_pct)}
				{formatDecimalString(risk.day_pnl_pct, { signed: true, suffix: '%' })}
			</span>
		</div>

		<div>
			<div class="mb-1.5 flex justify-between font-mono tabular-nums">
				<span class="text-ink-muted">
					Drawdown <span class="text-ink-dim">(kill {formatDecimalString(risk.limits.kill_pct)}%)</span>
				</span>
				<span class="text-warning">{formatDecimalString(risk.drawdown_pct, { suffix: '%' })}</span>
			</div>
			<div class="h-1.5 overflow-hidden rounded-full bg-surface-inset">
				<div
					class={`h-full transition-[width] duration-200 ${
						ddBar >= 70 ? 'bg-danger-solid' : 'bg-warning'
					}`}
					style={`width: ${ddBar}%`}
				></div>
			</div>
		</div>

		<div>
			<div class="mb-1.5 flex justify-between font-mono tabular-nums">
				<span class="text-ink-muted">Open Positions</span>
				<span class="text-ink-strong">{risk.open_positions} / {risk.max_positions}</span>
			</div>
			<div class="h-1.5 overflow-hidden rounded-full bg-surface-inset">
				<div
					class="h-full bg-accent transition-[width] duration-200"
					style={`width: ${posBar}%`}
				></div>
			</div>
		</div>
	</div>
</Panel>

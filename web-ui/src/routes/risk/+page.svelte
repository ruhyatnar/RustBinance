<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import HoldToConfirm from '$lib/components/HoldToConfirm.svelte';
	import RiskGauge from '$lib/components/RiskGauge.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';
	import { formatDecimalString } from '$lib/format/decimal';

	const snap = $derived(botStream.snapshot);
	let feedback = $state<string | null>(null);

	async function handleResetKill(): Promise<void> {
		try {
			const res = await botStream.executeConfirmedAction('reset_kill');
			feedback = res.message;
		} catch (err) {
			feedback = err instanceof Error ? err.message : 'reset_kill failed';
		}
	}
</script>

<div class="space-y-5">
	<PageHeader title="Risk" subtitle="Portfolio heat, guards, and circuit breaker thresholds" />

	{#if !snap}
		<Panel dotGrid={true}>
			<p class="py-8 text-center font-mono text-xs text-ink-dim">Waiting for risk snapshot…</p>
		</Panel>
	{:else}
		<RiskGauge risk={snap.risk} />

		<Panel title="Circuit Breaker & Limits" subtitle="Enforced server-side">
			<div class="grid grid-cols-1 gap-3 font-mono text-xs tabular-nums sm:grid-cols-3">
				<div class="rounded border border-hairline-subtle bg-surface-inset p-3.5">
					<div class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">
						Daily Halt Limit
					</div>
					<div class="mt-1.5 font-display text-lg font-semibold text-warning">
						{formatDecimalString(snap.risk.limits.daily_halt_pct, { suffix: '%' })}
					</div>
				</div>
				<div class="rounded border border-hairline-subtle bg-surface-inset p-3.5">
					<div class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">
						Drawdown Kill Limit
					</div>
					<div class="mt-1.5 font-display text-lg font-semibold text-danger">
						{formatDecimalString(snap.risk.limits.kill_pct, { suffix: '%' })}
					</div>
				</div>
				<div class="rounded border border-hairline-subtle bg-surface-inset p-3.5">
					<div class="text-[10px] tracking-[0.08em] text-ink-muted uppercase">
						Engine Mode
					</div>
					<div class="mt-1.5 font-display text-lg font-semibold text-ink-strong">
						{snap.engine.mode}
					</div>
				</div>
			</div>

			{#if botStream.canMutate && snap.engine.mode === 'Killed'}
				<div class="mt-4 flex flex-wrap items-center gap-3 border-t border-hairline-subtle pt-4 text-xs">
					<HoldToConfirm
						label="RESET KILL"
						variant="warning"
						holdMs={1500}
						onConfirm={handleResetKill}
					/>
					<span class="text-ink-muted">Only reset after root cause is verified.</span>
				</div>
			{/if}

			{#if feedback}
				<p class="mt-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 font-mono text-xs text-warning">
					{feedback}
				</p>
			{/if}
		</Panel>
	{/if}
</div>

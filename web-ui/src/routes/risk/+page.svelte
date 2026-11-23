<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import HoldToConfirm from '$lib/components/HoldToConfirm.svelte';
	import RiskGauge from '$lib/components/RiskGauge.svelte';
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

{#if snap}
	<div class="space-y-4">
		<RiskGauge risk={snap.risk} />

		<section class="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-xs">
			<h2 class="mb-3 font-bold tracking-wider text-slate-400 uppercase">
				Circuit Breaker & Limits
			</h2>
			<div class="grid grid-cols-1 gap-3 font-mono sm:grid-cols-3">
				<div class="rounded bg-slate-950/70 p-3">
					<div class="text-slate-400">Daily Halt Limit</div>
					<div class="mt-1 text-base font-bold text-amber-300">
						{formatDecimalString(snap.risk.limits.daily_halt_pct, { suffix: '%' })}
					</div>
				</div>
				<div class="rounded bg-slate-950/70 p-3">
					<div class="text-slate-400">Drawdown Kill Limit</div>
					<div class="mt-1 text-base font-bold text-red-400">
						{formatDecimalString(snap.risk.limits.kill_pct, { suffix: '%' })}
					</div>
				</div>
				<div class="rounded bg-slate-950/70 p-3">
					<div class="text-slate-400">Engine Mode</div>
					<div class="mt-1 text-base font-bold text-white">{snap.engine.mode}</div>
				</div>
			</div>

			{#if botStream.canMutate && snap.engine.mode === 'Killed'}
				<div class="mt-4 flex items-center gap-3">
					<HoldToConfirm
						label="RESET KILL"
						variant="warning"
						holdMs={1500}
						onConfirm={handleResetKill}
					/>
					<span class="text-slate-400">Only reset after root cause is verified.</span>
				</div>
			{/if}
			{#if feedback}
				<p class="mt-2 text-amber-300">{feedback}</p>
			{/if}
		</section>
	</div>
{/if}

<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';

	let paramKey = $state('risk.risk_per_trade_pct');
	let paramValue = $state('0.25');
	let feedback = $state<string | null>(null);

	async function handleTighten(ev: SubmitEvent): Promise<void> {
		ev.preventDefault();
		feedback = null;
		try {
			const res = await botStream.setParam(paramKey, paramValue);
			feedback = res.message;
		} catch (err) {
			feedback = err instanceof Error ? err.message : 'set_param rejected';
		}
	}
</script>

<section class="max-w-xl rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-xs">
	<h2 class="mb-2 font-bold tracking-wider text-slate-400 uppercase">
		Runtime Risk Parameters (Tighten-Only)
	</h2>
	<p class="mb-4 text-slate-400">
		Parameters can only be tightened at runtime (§7.8, Hard Rule #8). Any attempt to loosen risk
		limits is rejected by the server.
	</p>

	{#if !botStream.canMutate}
		<p class="rounded bg-slate-950/70 p-3 text-amber-300">
			Unlock controls with a full bearer token to submit tighten-only parameter updates.
		</p>
	{:else}
		<form onsubmit={handleTighten} class="space-y-3">
			<div>
				<label for="param-key" class="block text-slate-300">Parameter Key</label>
				<select
					id="param-key"
					bind:value={paramKey}
					class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2.5 py-2 font-mono text-slate-100"
				>
					<option value="risk.risk_per_trade_pct">risk.risk_per_trade_pct</option>
					<option value="risk.max_open_positions">risk.max_open_positions</option>
					<option value="risk.portfolio_heat_max_pct">risk.portfolio_heat_max_pct</option>
					<option value="risk.max_notional_per_position_pct">risk.max_notional_per_position_pct</option>
					<option value="risk.max_total_exposure_pct">risk.max_total_exposure_pct</option>
					<option value="risk.daily_loss_halt_pct">risk.daily_loss_halt_pct</option>
					<option value="risk.drawdown_kill_pct">risk.drawdown_kill_pct</option>
				</select>
			</div>

			<div>
				<label for="param-val" class="block text-slate-300">Tightened Value</label>
				<input
					id="param-val"
					type="text"
					bind:value={paramValue}
					required
					class="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2.5 py-2 font-mono text-slate-100"
				/>
			</div>

			<button
				type="submit"
				class="rounded-md bg-sky-600 px-3 py-2 font-semibold text-white hover:bg-sky-500"
			>
				Apply Tighten-Only Update
			</button>

			{#if feedback}
				<p class="rounded bg-slate-950/70 p-2 font-mono text-amber-300">{feedback}</p>
			{/if}
		</form>
	{/if}
</section>

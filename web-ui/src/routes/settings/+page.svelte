<script lang="ts">
	import { botStream } from '$lib/api/stream.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import Panel from '$lib/components/ui/Panel.svelte';

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

<div class="space-y-5">
	<PageHeader title="Settings" subtitle="Runtime risk overrides (strict tighten-only policy)" />

	<div class="max-w-xl">
		<Panel title="Runtime Risk Parameters" subtitle="Tighten-Only (§7.8)">
			<p class="mb-4 text-xs leading-relaxed text-ink-muted">
				Parameters can only be tightened at runtime (§7.8, Hard Rule #8). Any attempt to loosen risk
				limits is rejected by the server.
			</p>

			{#if !botStream.canMutate}
				<div class="rounded border border-warning/40 bg-warning/10 px-3.5 py-3 font-mono text-xs text-warning">
					🔒 Unlock controls with a full bearer token to submit tighten-only parameter updates.
				</div>
			{:else}
				<form onsubmit={handleTighten} class="space-y-4 text-xs">
					<div>
						<label
							for="param-key"
							class="block font-mono text-[11px] font-medium tracking-wider text-ink-muted uppercase"
						>
							Parameter Key
						</label>
						<select
							id="param-key"
							bind:value={paramKey}
							class="mt-1.5 w-full rounded border border-hairline bg-surface-inset px-3 py-2 font-mono text-xs text-ink-strong focus:border-accent focus:outline-none"
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
						<label
							for="param-val"
							class="block font-mono text-[11px] font-medium tracking-wider text-ink-muted uppercase"
						>
							Tightened Value
						</label>
						<input
							id="param-val"
							type="text"
							bind:value={paramValue}
							required
							class="mt-1.5 w-full rounded border border-hairline bg-surface-inset px-3 py-2 font-mono text-xs text-ink-strong focus:border-accent focus:outline-none"
						/>
					</div>

					<button
						type="submit"
						class="rounded bg-accent px-4 py-2 font-mono text-xs font-semibold text-white transition hover:bg-accent/90"
					>
						Apply Tighten-Only Update
					</button>

					{#if feedback}
						<p class="rounded border border-warning/40 bg-warning/10 px-3 py-2 font-mono text-xs text-warning">
							{feedback}
						</p>
					{/if}
				</form>
			{/if}
		</Panel>
	</div>
</div>

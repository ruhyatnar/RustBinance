<script lang="ts">
	import type { PositionView } from '$lib/api/types';
	import Panel from '$lib/components/ui/Panel.svelte';
	import {
		decimalDirectionIcon,
		decimalSign,
		formatDecimalString,
		formatDuration
	} from '$lib/format/decimal';

	interface Props {
		positions: PositionView[];
		canMutate: boolean;
		onClosePosition: (positionId: string) => void | Promise<void>;
	}

	let { positions, canMutate, onClosePosition }: Props = $props();
</script>

<Panel title={`Positions (${positions.length})`} padding="none">
	{#if positions.length === 0}
		<p class="py-8 text-center font-mono text-xs text-ink-dim">No open positions.</p>
	{:else}
		<div class="overflow-x-auto">
			<table class="w-full text-left font-mono text-xs tabular-nums">
				<thead class="border-b border-hairline-subtle bg-surface-inset/70 text-[10px] tracking-[0.08em] text-ink-muted uppercase">
					<tr>
						<th class="px-4 py-2.5">Symbol</th>
						<th class="px-3 py-2.5">Venue</th>
						<th class="px-3 py-2.5">PnL (R)</th>
						<th class="px-3 py-2.5">Unrealized</th>
						<th class="px-3 py-2.5">Entry / Mark</th>
						<th class="px-3 py-2.5">SL / TP1</th>
						<th class="px-3 py-2.5">Age</th>
						{#if canMutate}
							<th class="px-4 py-2.5 text-right">Action</th>
						{/if}
					</tr>
				</thead>
				<tbody class="divide-y divide-hairline-subtle">
					{#each positions as pos (pos.id)}
						{@const sign = decimalSign(pos.profit_r)}
						<tr class="transition hover:bg-surface-elevated/60">
							<td class="px-4 py-2.5 font-semibold text-ink-strong">
								<div>{pos.symbol}</div>
								<div class="mt-0.5 text-[10px] font-normal text-ink-dim">
									{pos.strategy} · {pos.state}
								</div>
							</td>
							<td class="px-3 py-2.5">
								<span class="rounded border border-hairline-strong bg-surface-inset px-1.5 py-0.5 text-[10px] font-bold text-ink uppercase">
									{pos.venue.toUpperCase()}
								</span>
							</td>
							<td
								class={`px-3 py-2.5 font-bold ${
									sign > 0 ? 'text-positive' : sign < 0 ? 'text-danger' : 'text-ink-muted'
								}`}
							>
								{decimalDirectionIcon(pos.profit_r)}
								{formatDecimalString(pos.profit_r, { signed: true, suffix: 'R' })}
							</td>
							<td class="px-3 py-2.5 text-ink">
								{formatDecimalString(pos.unrealized_quote, { signed: true })}
							</td>
							<td class="px-3 py-2.5">
								<div class="text-ink-strong">{formatDecimalString(pos.entry)}</div>
								<div class="text-[10px] text-ink-muted">
									{formatDecimalString(pos.mark)}
								</div>
							</td>
							<td class="px-3 py-2.5">
								<div class="flex items-center gap-1.5 text-ink">
									<span>{formatDecimalString(pos.stop)}</span>
									{#if pos.be_done}
										<span class="rounded border border-positive/40 bg-positive/10 px-1 text-[10px] text-positive">
											BE
										</span>
									{/if}
								</div>
								<div class="mt-0.5 flex items-center gap-1.5 text-[10px] text-ink-muted">
									<span>{formatDecimalString(pos.tp1)}</span>
									{#if pos.tp1_done}
										<span class="rounded border border-telemetry/40 bg-telemetry/10 px-1 text-telemetry">
											TP1
										</span>
									{/if}
								</div>
							</td>
							<td class="px-3 py-2.5 text-ink-dim">{formatDuration(pos.age_s)}</td>
							{#if canMutate}
								<td class="px-4 py-2.5 text-right">
									<button
										type="button"
										onclick={() => onClosePosition(pos.id)}
										class="rounded border border-danger-solid/60 bg-danger-solid/15 px-2.5 py-1 text-[11px] font-semibold text-danger transition hover:bg-danger-solid/30"
									>
										Close
									</button>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</Panel>

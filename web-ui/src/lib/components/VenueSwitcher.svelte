<script lang="ts">
	import type { Venue, VenueState, VenuesSummary } from '$lib/api/types';

	export interface Props {
		venues: VenuesSummary;
		readOnly?: boolean;
		drainingOpenPositions?: number;
		compact?: boolean;
		onRequestSwitch?: (target: Venue) => void;
	}

	let {
		venues,
		readOnly = false,
		drainingOpenPositions = 0,
		compact = false,
		onRequestSwitch
	}: Props = $props();

	function stateBadge(st: VenueState): string {
		switch (st) {
			case 'Active':
				return '✓';
			case 'Draining':
				return '⏳';
			case 'Activating':
				return '…';
			case 'Disabled':
				return '–';
		}
	}

	const drainingVenue = $derived(
		venues.spot === 'Draining'
			? 'Spot'
			: venues.futures === 'Draining'
				? 'Futures'
				: null
	);
</script>

{#if !readOnly}
	<div class="flex flex-col gap-2" data-testid="venue-switcher">
		<div
			class={`grid gap-1 rounded-md border border-hairline bg-surface-inset p-1 ${
				compact ? 'grid-cols-1' : 'inline-grid grid-cols-2'
			}`}
			role="group"
			aria-label="Venue Switcher"
		>
			<button
				type="button"
				data-testid="venue-btn-spot"
				onclick={() => onRequestSwitch?.('spot')}
				class={`flex items-center justify-between gap-1.5 rounded px-2.5 py-1.5 text-left font-mono text-[11px] font-semibold transition ${
					venues.active === 'spot'
						? 'border border-accent/60 bg-surface-elevated text-ink-strong'
						: 'border border-transparent text-ink-muted hover:bg-surface hover:text-ink'
				}`}
			>
				<span>SPOT</span>
				<span class="text-[10px] text-ink-dim">
					{venues.spot} {stateBadge(venues.spot)}
				</span>
			</button>

			<button
				type="button"
				data-testid="venue-btn-futures"
				onclick={() => onRequestSwitch?.('futures')}
				class={`flex flex-col gap-0.5 rounded px-2.5 py-1.5 text-left font-mono text-[11px] font-semibold transition ${
					venues.active === 'futures'
						? 'border border-accent/60 bg-surface-elevated text-ink-strong'
						: 'border border-transparent text-ink-muted hover:bg-surface hover:text-ink'
				}`}
			>
				<span class="flex items-center justify-between gap-1.5">
					<span>FUTURES</span>
					<span class="text-[10px] text-ink-dim">
						{venues.futures} {stateBadge(venues.futures)}
					</span>
				</span>
				<span class="text-[10px] font-normal text-warning/90">ISOLATED · maks 3×</span>
			</button>
		</div>

		{#if drainingVenue}
			<div
				class="rounded border border-warning/40 bg-warning/10 px-2.5 py-1.5 font-mono text-[11px] text-warning"
				role="status"
				data-testid="draining-banner"
			>
				⏳ {drainingVenue} sedang draining: {drainingOpenPositions} posisi dikelola, tidak ada entry baru
			</div>
		{/if}
	</div>
{/if}

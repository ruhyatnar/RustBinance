<script lang="ts">
	import type { Venue, VenueState, VenuesSummary } from '$lib/api/types';

	export interface Props {
		venues: VenuesSummary;
		readOnly?: boolean;
		drainingOpenPositions?: number;
		onRequestSwitch?: (target: Venue) => void;
	}

	let {
		venues,
		readOnly = false,
		drainingOpenPositions = 0,
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
	<div class="venue-switcher" data-testid="venue-switcher">
		<div class="segmented" role="group" aria-label="Venue Switcher">
			<button
				type="button"
				class:active={venues.active === 'spot'}
				data-testid="venue-btn-spot"
				onclick={() => onRequestSwitch?.('spot')}
			>
				SPOT <span class="badge">{venues.spot} {stateBadge(venues.spot)}</span>
			</button>
			<button
				type="button"
				class:active={venues.active === 'usds_futures'}
				data-testid="venue-btn-futures"
				onclick={() => onRequestSwitch?.('usds_futures')}
			>
				FUTURES <span class="badge">{venues.futures} {stateBadge(venues.futures)}</span>
				<span class="meta">ISOLATED · maks 3×</span>
			</button>
		</div>

		{#if drainingVenue}
			<div class="draining-banner" role="status" data-testid="draining-banner">
				{drainingVenue} sedang draining: {drainingOpenPositions} posisi dikelola, tidak ada entry baru
			</div>
		{/if}
	</div>
{/if}

<style>
	.venue-switcher {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}
	.segmented {
		display: inline-flex;
		gap: 0.25rem;
		border: 1px solid #334155;
		border-radius: 0.5rem;
		padding: 0.2rem;
	}
	button {
		background: transparent;
		color: #cbd5e1;
		border: none;
		padding: 0.35rem 0.75rem;
		border-radius: 0.35rem;
		cursor: pointer;
		font-weight: 600;
	}
	button.active {
		background: #1e293b;
		color: #f8fafc;
	}
	.meta {
		margin-left: 0.35rem;
		font-size: 0.75rem;
		color: #94a3b8;
	}
	.draining-banner {
		font-size: 0.85rem;
		padding: 0.35rem 0.6rem;
		border-radius: 0.35rem;
		background: #422006;
		color: #fde68a;
	}
</style>

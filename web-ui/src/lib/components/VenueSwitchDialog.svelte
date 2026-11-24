<script lang="ts">
	import type { EngineMode, Venue } from '$lib/api/types';

	export type SwitchPolicyOption = 'drain' | 'refuse' | 'flatten';

	export interface Props {
		open: boolean;
		currentVenue: Venue;
		targetVenue: Venue;
		engineMode: EngineMode;
		openPositionsCount: number;
		openPnlUsd?: string;
		rejectionReason?: string | null;
		onConfirm?: (policy: SwitchPolicyOption) => void;
		onCancel?: () => void;
	}

	let {
		open,
		currentVenue,
		targetVenue,
		engineMode,
		openPositionsCount,
		openPnlUsd = '0.00',
		rejectionReason = null,
		onConfirm,
		onCancel
	}: Props = $props();

	let policy = $state<SwitchPolicyOption>('drain');

	const blockedReason = $derived(
		rejectionReason ??
			(engineMode === 'Halted' || engineMode === 'Killed'
				? `Peralihan ditolak saat engine dalam mode ${engineMode}`
				: null)
	);
</script>

{#if open}
	<div class="dialog-backdrop" role="dialog" aria-modal="true" data-testid="venue-switch-dialog">
		<div class="dialog">
			<h3>Alihkan Venue: {currentVenue} → {targetVenue}</h3>
			<p>Posisi terbuka saat ini: <strong>{openPositionsCount}</strong> (PnL: {openPnlUsd} USD)</p>

			{#if targetVenue === 'UsdsFutures'}
				<p class="notice">Futures: ISOLATED · maks 3× · mendukung Short · funding & likuidasi aktif.</p>
			{/if}

			<fieldset>
				<legend>Kebijakan Posisi Terbuka</legend>
				<label>
					<input type="radio" name="policy" value="drain" bind:group={policy} />
					Kelola sampai selesai (Draining — default)
				</label>
				<label>
					<input type="radio" name="policy" value="refuse" bind:group={policy} />
					Wajib flat dulu
				</label>
				<label>
					<input type="radio" name="policy" value="flatten" bind:group={policy} />
					Tutup semua sekarang (tahan 1,5 detik + nonce)
				</label>
			</fieldset>

			{#if blockedReason}
				<p class="error" data-testid="switch-blocked-reason">{blockedReason}</p>
			{/if}

			<div class="actions">
				<button type="button" onclick={() => onCancel?.()}>Batal</button>
				<button
					type="button"
					disabled={Boolean(blockedReason)}
					data-testid="confirm-switch-btn"
					onclick={() => onConfirm?.(policy)}
				>
					{policy === 'flatten' ? 'Tahan 1,5s untuk Flatten & Alihkan' : 'Alihkan Venue'}
				</button>
			</div>
		</div>
	</div>
{/if}

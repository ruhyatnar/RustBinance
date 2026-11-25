<script lang="ts">
	import type { EngineMode, Venue } from '$lib/api/types';
	import HoldToConfirm from './HoldToConfirm.svelte';

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
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm"
		role="dialog"
		aria-modal="true"
		data-testid="venue-switch-dialog"
	>
		<div class="inner-glow w-full max-w-lg rounded-lg border border-hairline bg-surface p-5 shadow-2xl">
			<div class="flex items-start justify-between gap-3 border-b border-hairline-subtle pb-3">
				<div>
					<p class="font-mono text-[10px] font-semibold tracking-[0.12em] text-telemetry uppercase">
						Venue Switch Protocol
					</p>
					<h3 class="mt-1 font-display text-lg font-semibold tracking-[-0.02em] text-ink-strong uppercase">
						Alihkan Venue: {currentVenue} → {targetVenue}
					</h3>
				</div>
				<button
					type="button"
					onclick={() => onCancel?.()}
					aria-label="Tutup dialog"
					class="rounded border border-hairline bg-surface-inset px-2 py-0.5 text-xs text-ink-muted hover:text-ink-strong"
				>
					✕
				</button>
			</div>

			<div class="mt-4 space-y-3 text-xs">
				<div class="flex items-center justify-between rounded border border-hairline-subtle bg-surface-inset px-3 py-2 font-mono tabular-nums">
					<span class="text-ink-muted">Posisi terbuka saat ini</span>
					<span class="text-ink-strong">
						<strong>{openPositionsCount}</strong> (PnL: {openPnlUsd} USD)
					</span>
				</div>

				{#if targetVenue === 'futures'}
					<p class="rounded border border-warning/40 bg-warning/10 px-3 py-2 text-warning">
						⚠ Futures: ISOLATED · maks 3× · mendukung Short · funding & likuidasi aktif.
					</p>
				{/if}

				<fieldset class="space-y-2">
					<legend class="mb-1.5 font-mono text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
						Kebijakan Posisi Terbuka
					</legend>

					<label
						class={`flex cursor-pointer items-start gap-2.5 rounded border p-2.5 transition ${
							policy === 'drain'
								? 'border-accent/60 bg-surface-elevated text-ink-strong'
								: 'border-hairline-subtle bg-surface-inset text-ink-muted hover:border-hairline'
						}`}
					>
						<input
							type="radio"
							name="policy"
							value="drain"
							bind:group={policy}
							class="mt-0.5 accent-accent"
						/>
						<span>Kelola sampai selesai (Draining — default)</span>
					</label>

					<label
						class={`flex cursor-pointer items-start gap-2.5 rounded border p-2.5 transition ${
							policy === 'refuse'
								? 'border-accent/60 bg-surface-elevated text-ink-strong'
								: 'border-hairline-subtle bg-surface-inset text-ink-muted hover:border-hairline'
						}`}
					>
						<input
							type="radio"
							name="policy"
							value="refuse"
							bind:group={policy}
							class="mt-0.5 accent-accent"
						/>
						<span>Wajib flat dulu</span>
					</label>

					<label
						class={`flex cursor-pointer items-start gap-2.5 rounded border p-2.5 transition ${
							policy === 'flatten'
								? 'border-danger-solid/70 bg-danger-solid/10 text-danger'
								: 'border-hairline-subtle bg-surface-inset text-ink-muted hover:border-hairline'
						}`}
					>
						<input
							type="radio"
							name="policy"
							value="flatten"
							bind:group={policy}
							class="mt-0.5 accent-danger-solid"
						/>
						<span>Tutup semua sekarang (tahan 1,5 detik + nonce)</span>
					</label>
				</fieldset>

				{#if blockedReason}
					<p
						class="rounded border border-danger-solid/60 bg-danger-solid/15 px-3 py-2 font-mono text-danger"
						data-testid="switch-blocked-reason"
					>
						⚠ {blockedReason}
					</p>
				{/if}
			</div>

			<div class="mt-5 flex items-center justify-end gap-2 border-t border-hairline-subtle pt-3">
				<button
					type="button"
					onclick={() => onCancel?.()}
					class="rounded border border-hairline bg-surface-inset px-3 py-1.5 font-mono text-xs text-ink-muted hover:border-hairline-strong hover:text-ink-strong"
				>
					Batal
				</button>

				{#if policy === 'flatten'}
					<HoldToConfirm
						label="Flatten & Alihkan"
						holdMs={1500}
						variant="danger"
						disabled={Boolean(blockedReason)}
						onConfirm={() => onConfirm?.('flatten')}
					/>
				{:else}
					<button
						type="button"
						disabled={Boolean(blockedReason)}
						data-testid="confirm-switch-btn"
						onclick={() => onConfirm?.(policy)}
						class="rounded border border-accent bg-accent px-3.5 py-1.5 font-mono text-xs font-semibold text-white transition hover:bg-accent/90 disabled:cursor-not-allowed disabled:opacity-45"
					>
						Alihkan Venue
					</button>
				{/if}
			</div>
		</div>
	</div>
{/if}

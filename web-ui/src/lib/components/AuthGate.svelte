<script lang="ts">
	import { untrack } from 'svelte';

	interface Props {
		defaultUrl?: string;
		errorMessage?: string | null;
		onConnect: (baseUrl: string, token: string, persistSession: boolean) => Promise<void>;
	}

	let {
		defaultUrl = 'http://127.0.0.1:3001',
		errorMessage = null,
		onConnect
	}: Props = $props();

	let baseUrl = $state(untrack(() => defaultUrl));
	let token = $state('');
	let persistSession = $state(true);
	let busy = $state(false);
	let localError = $state<string | null>(null);

	async function handleSubmit(ev: SubmitEvent): Promise<void> {
		ev.preventDefault();
		localError = null;
		if (!token.trim()) {
			localError = 'Bearer token is required.';
			return;
		}
		busy = true;
		try {
			await onConnect(baseUrl, token, persistSession);
			token = '';
		} catch (err) {
			localError = err instanceof Error ? err.message : 'Authentication failed';
		} finally {
			busy = false;
		}
	}
</script>

<div class="dot-grid flex min-h-screen items-center justify-center bg-canvas p-4 text-ink">
	<div class="inner-glow w-full max-w-md rounded-lg border border-hairline bg-surface p-6 shadow-2xl">
		<div class="mb-5 flex items-start justify-between border-b border-hairline-subtle pb-4">
			<div>
				<p class="font-mono text-[10px] font-semibold tracking-[0.14em] text-telemetry uppercase">
					◈ Binance Rust Trading Bot
				</p>
				<h1 class="mt-1 font-display text-xl font-semibold tracking-[-0.03em] text-ink-strong">
					Local Control Gateway
				</h1>
			</div>
			<span class="rounded border border-hairline-strong bg-surface-inset px-2 py-0.5 font-mono text-[10px] font-semibold text-accent-soft uppercase">
				IPC v1
			</span>
		</div>

		<p class="mb-5 text-xs leading-relaxed text-ink-muted">
			Enter your local bearer token. Read-only tokens unlock live monitoring only; full tokens
			start in view-only mode until controls are explicitly unlocked. Tokens are never saved in
			<code class="rounded bg-surface-inset px-1 py-0.5 text-ink">localStorage</code>.
		</p>

		<form onsubmit={handleSubmit} class="space-y-4">
			<div>
				<label for="ipc-url" class="block font-mono text-[11px] font-medium tracking-wider text-ink-muted uppercase">
					Gateway URL (loopback)
				</label>
				<input
					id="ipc-url"
					type="url"
					bind:value={baseUrl}
					required
					class="mt-1.5 w-full rounded border border-hairline bg-surface-inset px-3 py-2 font-mono text-xs text-ink-strong focus:border-accent focus:outline-none"
				/>
			</div>

			<div>
				<label for="ipc-token" class="block font-mono text-[11px] font-medium tracking-wider text-ink-muted uppercase">
					Bearer Token
				</label>
				<input
					id="ipc-token"
					type="password"
					autocomplete="off"
					bind:value={token}
					required
					placeholder="Paste read-only or full bearer token"
					class="mt-1.5 w-full rounded border border-hairline bg-surface-inset px-3 py-2 font-mono text-xs text-ink-strong placeholder:text-ink-dim focus:border-accent focus:outline-none"
				/>
			</div>

			<label class="flex items-center gap-2 text-xs text-ink-muted">
				<input type="checkbox" bind:checked={persistSession} class="rounded border-hairline bg-surface-inset accent-accent" />
				<span>
					Keep token in <code class="text-ink">sessionStorage</code> for this tab only
				</span>
			</label>

			{#if localError || errorMessage}
				<div
					role="alert"
					class="rounded border border-danger-solid/60 bg-danger-solid/15 px-3 py-2 font-mono text-xs text-danger"
				>
					⚠ {localError ?? errorMessage}
				</div>
			{/if}

			<button
				type="submit"
				disabled={busy}
				class="w-full rounded bg-accent px-4 py-2.5 font-display text-sm font-semibold text-white transition hover:bg-accent/90 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{busy ? 'Connecting…' : 'Connect'}
			</button>
		</form>
	</div>
</div>

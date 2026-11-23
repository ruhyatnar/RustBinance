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

<div class="flex min-h-screen items-center justify-center bg-slate-950 p-4 text-slate-100">
	<div class="w-full max-w-md rounded-xl border border-slate-800 bg-slate-900/90 p-6 shadow-2xl">
		<div class="mb-5 flex items-center justify-between">
			<div>
				<p class="text-xs font-semibold tracking-wider text-sky-400 uppercase">
					Binance Rust Trading Bot
				</p>
				<h1 class="mt-1 text-xl font-bold text-white">Local Control Gateway</h1>
			</div>
			<span class="rounded bg-slate-800 px-2 py-1 text-xs font-medium text-slate-300">
				IPC v1
			</span>
		</div>

		<p class="mb-4 text-sm text-slate-400">
			Enter your local bearer token. Read-only tokens unlock live monitoring only; full tokens
			start in view-only mode until controls are explicitly unlocked. Tokens are never saved in
			<code class="text-slate-200">localStorage</code>.
		</p>

		<form onsubmit={handleSubmit} class="space-y-4">
			<div>
				<label for="ipc-url" class="block text-xs font-medium text-slate-300">
					Gateway URL (loopback)
				</label>
				<input
					id="ipc-url"
					type="url"
					bind:value={baseUrl}
					required
					class="mt-1 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 focus:border-sky-500 focus:outline-none"
				/>
			</div>

			<div>
				<label for="ipc-token" class="block text-xs font-medium text-slate-300">
					Bearer Token
				</label>
				<input
					id="ipc-token"
					type="password"
					autocomplete="off"
					bind:value={token}
					required
					placeholder="Paste read-only or full bearer token"
					class="mt-1 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 focus:border-sky-500 focus:outline-none"
				/>
			</div>

			<label class="flex items-center gap-2 text-xs text-slate-400">
				<input type="checkbox" bind:checked={persistSession} class="rounded border-slate-700" />
				Keep token in <code class="text-slate-300">sessionStorage</code> for this tab only
			</label>

			{#if localError || errorMessage}
				<div
					role="alert"
					class="rounded-lg border border-red-800/80 bg-red-950/60 px-3 py-2 text-xs text-red-200"
				>
					{localError ?? errorMessage}
				</div>
			{/if}

			<button
				type="submit"
				disabled={busy}
				class="w-full rounded-lg bg-sky-600 px-4 py-2.5 text-sm font-semibold text-white transition hover:bg-sky-500 disabled:opacity-50"
			>
				{busy ? 'Connecting…' : 'Connect'}
			</button>
		</form>
	</div>
</div>

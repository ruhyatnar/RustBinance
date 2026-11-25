import { describe, expect, it, vi } from 'vitest';
import { ApiError, IpcHttpClient } from './client';

describe('IpcHttpClient', () => {
	it('sends Authorization Bearer header (never query string) and parses auth_scope from health', async () => {
		const fetchMock = vi.fn(async () =>
			new Response(
				JSON.stringify({
					status: 'ok',
					engine_mode: 'Ready',
					auth_scope: 'readonly',
					ts_ms: 1790000000000
				}),
				{ status: 200, headers: { 'content-type': 'application/json' } }
			)
		);

		const client = new IpcHttpClient('http://127.0.0.1:3001', 'ro-token-abc', fetchMock);
		const health = await client.health();

		expect(health.auth_scope).toBe('readonly');
		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('http://127.0.0.1:3001/api/v1/health');
		expect(url).not.toContain('token=');
		expect((init.headers as Record<string, string>)['Authorization']).toBe('Bearer ro-token-abc');
	});

	it('normalizes both HttpErrorResponse and CommandResultPayload failures into ApiError', async () => {
		const httpErrFetch = vi.fn(async () =>
			new Response(
				JSON.stringify({ error: { code: 'forbidden', message: 'read-only token' } }),
				{ status: 403, headers: { 'content-type': 'application/json' } }
			)
		);
		const client1 = new IpcHttpClient('http://127.0.0.1:3001', 'ro-token', httpErrFetch);
		await expect(client1.pause()).rejects.toMatchObject({
			name: 'ApiError',
			status: 403,
			code: 'forbidden',
			message: 'read-only token'
		});

		const cmdErrFetch = vi.fn(async () =>
			new Response(
				JSON.stringify({
					cmd_id: '0192f0a8-1234-7000-8000-000000000001',
					ok: false,
					code: 'loosen_rejected',
					message: 'cannot loosen risk_per_trade_pct'
				}),
				{ status: 400, headers: { 'content-type': 'application/json' } }
			)
		);
		const client2 = new IpcHttpClient('http://127.0.0.1:3001', 'full-token', cmdErrFetch);
		await expect(client2.setParam('risk.risk_per_trade_pct', '1.0')).rejects.toBeInstanceOf(
			ApiError
		);
		await expect(client2.setParam('risk.risk_per_trade_pct', '1.0')).rejects.toMatchObject({
			status: 400,
			code: 'loosen_rejected'
		});
	});

	it('dispatches switchVenue to /api/v1/commands/switch-venue with policy and optional nonce', async () => {
		const fetchMock = vi.fn(async () =>
			new Response(
				JSON.stringify({
					cmd_id: '0192f0a8-1234-7000-8000-000000000002',
					ok: true,
					code: 'ok',
					message: 'switched Spot -> UsdsFutures'
				}),
				{ status: 200, headers: { 'content-type': 'application/json' } }
			)
		);
		const client = new IpcHttpClient('http://127.0.0.1:3001', 'full-token', fetchMock);
		const res = await client.switchVenue('futures', 'drain');
		expect(res.ok).toBe(true);
		const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('http://127.0.0.1:3001/api/v1/commands/switch-venue');
		expect(JSON.parse(String(init.body))).toMatchObject({
			target: 'futures',
			on_open_positions: 'drain',
			confirm_nonce: ''
		});
	});
});

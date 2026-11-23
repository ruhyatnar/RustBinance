import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { isHttpErrorResponse, isWireEnvelope } from './types';

const FIXTURES_DIR = resolve(process.cwd(), '../protocol/fixtures/v1');

function loadFixture(name: string): unknown {
	return JSON.parse(readFileSync(resolve(FIXTURES_DIR, name), 'utf-8'));
}

describe('IPC v1 protocol fixtures contract', () => {
	const envelopeFixtures = [
		'hello.json',
		'snapshot.json',
		'delta.json',
		'market.json',
		'event.json',
		'heartbeat.json',
		'command_result.json',
		'command_pause.json'
	];

	for (const file of envelopeFixtures) {
		it(`validates ${file} as a v1 WireEnvelope`, () => {
			const parsed = loadFixture(file);
			expect(isWireEnvelope(parsed)).toBe(true);
		});
	}

	it('validates http_error.json as HttpErrorResponse', () => {
		const parsed = loadFixture('http_error.json');
		expect(isHttpErrorResponse(parsed)).toBe(true);
	});
});

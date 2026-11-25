import { describe, expect, it } from 'vitest';
import { filterByVenue, filterEventsByVenue } from './venue';

describe('venue filter helpers', () => {
	const items = [
		{ id: '1', venue: 'spot' as const },
		{ id: '2', venue: 'futures' as const }
	];

	it('filterByVenue returns all items for "all" and narrows for "spot"/"futures"', () => {
		expect(filterByVenue(items, 'all')).toHaveLength(2);
		expect(filterByVenue(items, 'spot').map((i) => i.id)).toEqual(['1']);
		expect(filterByVenue(items, 'futures').map((i) => i.id)).toEqual(['2']);
	});

	it('filterEventsByVenue always preserves global system events with null/undefined venue', () => {
		const events = [
			{ seq: 1, venue: null, msg: 'global halt' },
			{ seq: 2, venue: undefined, msg: 'ws reconnect' },
			{ seq: 3, venue: 'spot' as const, msg: 'spot fill' },
			{ seq: 4, venue: 'futures' as const, msg: 'futures fill' }
		];

		expect(filterEventsByVenue(events, 'all').map((e) => e.seq)).toEqual([1, 2, 3, 4]);
		expect(filterEventsByVenue(events, 'futures').map((e) => e.seq)).toEqual([1, 2, 4]);
		expect(filterEventsByVenue(events, 'spot').map((e) => e.seq)).toEqual([1, 2, 3]);
	});
});

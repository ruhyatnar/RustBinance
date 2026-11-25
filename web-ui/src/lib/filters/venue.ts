import type { Venue } from '$lib/api/types';

export type VenueFilter = 'all' | Venue;

export function filterByVenue<T extends { venue: Venue }>(
	items: T[],
	filter: VenueFilter
): T[] {
	if (filter === 'all') return items;
	return items.filter((item) => item.venue === filter);
}

export function filterEventsByVenue<T extends { venue?: Venue | null }>(
	events: T[],
	filter: VenueFilter
): T[] {
	if (filter === 'all') return events;
	return events.filter((ev) => ev.venue == null || ev.venue === filter);
}

import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createMockRequest, parseResponse } from '../helpers/api';

const mockPrisma = vi.hoisted(() => ({
  user: {
    findMany: vi.fn(),
    count: vi.fn(),
  },
  $queryRaw: vi.fn(),
}));

vi.mock('@/lib/prisma', () => ({
  prisma: mockPrisma,
}));

// The route imports `Prisma` from '@prisma/client' for `Prisma.sql`. Provide a
// lightweight tagged-template stand-in so the module loads without the real
// generated client.
vi.mock('@prisma/client', () => ({
  Prisma: {
    sql: (strings: TemplateStringsArray, ...values: unknown[]) => ({
      strings,
      values,
    }),
  },
}));

import { GET } from '@/app/api/leaderboard/route';

function makeUser(overrides: Record<string, any> = {}) {
  return {
    id: 'user_1',
    name: 'Alice',
    avatarUrl: '/alice.png',
    xp: 100,
    rank: { id: 'gold', title: 'Gold', level: 3 },
    badges: [],
    _count: { posts: 2, entries: 3, helpContributions: 4 },
    ...overrides,
  };
}

describe('Leaderboard API', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('reports overall count in `total`, not the current page size', async () => {
    mockPrisma.user.count.mockResolvedValue(137);
    mockPrisma.user.findMany.mockResolvedValue([makeUser()]);

    const request = createMockRequest(
      'http://localhost:3000/api/leaderboard?page=1&limit=50',
    );
    const { status, data } = await parseResponse(await GET(request));

    expect(status).toBe(200);
    expect(data.success).toBe(true);
    // total is the overall count (137), even though the page has 1 row.
    expect(data.data.total).toBe(137);
    expect(data.data.leaderboard).toHaveLength(1);
    expect(data.data.total_pages).toBe(Math.ceil(137 / 50));
    expect(mockPrisma.user.count).toHaveBeenCalledTimes(1);
  });

  it('all-time period orders by xp with a stable id tiebreaker', async () => {
    mockPrisma.user.count.mockResolvedValue(1);
    mockPrisma.user.findMany.mockResolvedValue([makeUser()]);

    const request = createMockRequest(
      'http://localhost:3000/api/leaderboard?period=all-time',
    );
    await GET(request);

    expect(mockPrisma.$queryRaw).not.toHaveBeenCalled();
    const call = mockPrisma.user.findMany.mock.calls[0][0];
    expect(call.orderBy).toEqual([{ xp: 'desc' }, { id: 'asc' }]);
  });

  it('weekly period ranks by a window-scoped metric via SQL (not all-time xp)', async () => {
    mockPrisma.user.count.mockResolvedValue(2);
    // SQL returns ranked ids in period order: user_2 first, then user_1.
    mockPrisma.$queryRaw.mockResolvedValue([{ id: 'user_2' }, { id: 'user_1' }]);
    // findMany with `in` returns them in arbitrary order.
    mockPrisma.user.findMany.mockResolvedValue([
      makeUser({ id: 'user_1', xp: 999 }),
      makeUser({ id: 'user_2', xp: 1 }),
    ]);

    const request = createMockRequest(
      'http://localhost:3000/api/leaderboard?period=weekly',
    );
    const { status, data } = await parseResponse(await GET(request));

    expect(status).toBe(200);
    expect(mockPrisma.$queryRaw).toHaveBeenCalledTimes(1);
    // Result preserves the SQL ranking order, NOT all-time xp order.
    expect(data.data.leaderboard.map((u: any) => u.id)).toEqual([
      'user_2',
      'user_1',
    ]);
  });

  it('sorts badges by tier with capitalized values and includes Diamond', async () => {
    mockPrisma.user.count.mockResolvedValue(1);
    mockPrisma.user.findMany.mockResolvedValue([
      makeUser({
        badges: [
          { badge: { id: 'b1', name: 'Bronze Badge', tier: 'Bronze' } },
          { badge: { id: 'd1', name: 'Diamond Badge', tier: 'Diamond' } },
          { badge: { id: 'g1', name: 'Gold Badge', tier: 'Gold' } },
        ],
      }),
    ]);

    const request = createMockRequest('http://localhost:3000/api/leaderboard');
    const { data } = await parseResponse(await GET(request));

    const tiers = data.data.leaderboard[0].badges.map((b: any) => b.tier);
    // Highest tier first: Diamond > Gold > Bronze.
    expect(tiers).toEqual(['Diamond', 'Gold', 'Bronze']);
  });

  it('total_contributions includes help contributions', async () => {
    mockPrisma.user.count.mockResolvedValue(1);
    mockPrisma.user.findMany.mockResolvedValue([
      makeUser({ _count: { posts: 2, entries: 3, helpContributions: 4 } }),
    ]);

    const request = createMockRequest('http://localhost:3000/api/leaderboard');
    const { data } = await parseResponse(await GET(request));

    const row = data.data.leaderboard[0];
    expect(row.post_count).toBe(2);
    expect(row.entry_count).toBe(3);
    expect(row.help_contribution_count).toBe(4);
    expect(row.total_contributions).toBe(9);
  });
});

import { apiError, apiSuccess } from '@/lib/api-response';

import { NextRequest } from 'next/server';
import { Prisma } from '@prisma/client';
import { prisma } from '@/lib/prisma';
import { parsePagination } from '@/lib/pagination';

/**
 * Ordering weight for badge tiers.
 *
 * `Badge.tier` is a free-form string that mirrors the `BadgeTier` enum
 * (Bronze/Silver/Gold/Platinum/Diamond). Historically the values were looked
 * up with lowercase keys while the stored values are capitalized, so every
 * lookup returned `undefined` and the sort became a no-op (and Diamond was
 * missing entirely). Keys here are lowercase and lookups normalize casing.
 */
const tierOrder: Record<string, number> = {
  bronze: 1,
  silver: 2,
  gold: 3,
  platinum: 4,
  diamond: 5,
};

/**
 * Build the `select` used to hydrate a page of leaderboard users.
 *
 * When a `dateFilter` is supplied, the relation `_count`s are scoped to the
 * window. Note the different timestamp columns: posts/entries use `createdAt`
 * while help contributions use `contributedAt`.
 */
function buildUserSelect(dateFilter?: Date) {
  return {
    id: true,
    name: true,
    avatarUrl: true,
    xp: true,
    rank: true,
    badges: {
      include: { badge: true },
    },
    _count: {
      select: {
        posts: dateFilter
          ? { where: { createdAt: { gte: dateFilter } } }
          : true,
        entries: dateFilter
          ? { where: { createdAt: { gte: dateFilter } } }
          : true,
        helpContributions: dateFilter
          ? { where: { contributedAt: { gte: dateFilter } } }
          : true,
      },
    },
  } satisfies Prisma.UserSelect;
}

export async function GET (request: NextRequest) {
  try {
    const { searchParams } = new URL(request.url);
    const period = searchParams.get('period') || 'all-time';
    const { page, limit, skip } = parsePagination(searchParams, {
      defaultLimit: 50,
    });

    let dateFilter: Date | undefined;
    if (period === 'weekly') {
      dateFilter = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);
    } else if (period === 'monthly') {
      dateFilter = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000);
    }

    // `total` is the overall number of ranked users, independent of the current
    // page. No user-level filter is applied (period only scopes metrics), so the
    // count is over the same set the leaderboard ranks.
    const total = await prisma.user.count();

    const select = buildUserSelect(dateFilter);

    // Determine the ordered set of user ids for this page.
    // - all-time: rank by all-time xp.
    // - weekly/monthly: rank by contributions made *within the window* so the
    //   period boards are actually different from the all-time board. Prisma
    //   cannot `orderBy` a filtered relation count, so this ranking (and its
    //   pagination) is computed in SQL and then hydrated via `findMany`.
    let users: Prisma.UserGetPayload<{ select: ReturnType<typeof buildUserSelect> }>[];

    if (dateFilter) {
      const rankedIds = await prisma.$queryRaw<Array<{ id: string }>>(Prisma.sql`
        SELECT u.id
        FROM users u
        ORDER BY (
          (SELECT COUNT(*) FROM posts p
            WHERE p.creator_id = u.id AND p.created_at >= ${dateFilter})
          + (SELECT COUNT(*) FROM entries e
            WHERE e.user_id = u.id AND e.created_at >= ${dateFilter})
          + (SELECT COUNT(*) FROM help_contributions h
            WHERE h.user_id = u.id AND h.contributed_at >= ${dateFilter})
        ) DESC,
        u.xp DESC,
        u.id ASC
        LIMIT ${limit} OFFSET ${skip}
      `);

      const orderedIds = rankedIds.map((row) => row.id);

      const rows = orderedIds.length
        ? await prisma.user.findMany({
            where: { id: { in: orderedIds } },
            select,
          })
        : [];

      // Preserve the SQL ranking order (findMany `in` does not guarantee it).
      const byId = new Map(rows.map((row) => [row.id, row]));
      users = orderedIds
        .map((id) => byId.get(id))
        .filter((row): row is (typeof rows)[number] => row !== undefined);
    } else {
      users = await prisma.user.findMany({
        select,
        // Stable ordering with a deterministic tiebreaker.
        orderBy: [{ xp: 'desc' }, { id: 'asc' }],
        take: limit,
        skip,
      });
    }

    const leaderboard = users.map((user) => {
      const badges = user.badges
        .map((ub) => ub.badge)
        .sort(
          (a, b) =>
            (tierOrder[b.tier?.toLowerCase() ?? ''] || 0) -
            (tierOrder[a.tier?.toLowerCase() ?? ''] || 0)
        );

      return {
        id: user.id,
        name: user.name,
        avatar_url: user.avatarUrl,
        xp: user.xp,
        rank: user.rank,
        post_count: user._count.posts,
        entry_count: user._count.entries,
        help_contribution_count: user._count.helpContributions,
        total_contributions:
          user._count.posts +
          user._count.entries +
          user._count.helpContributions,
        badges,
      };
    });

    return apiSuccess({
      leaderboard,
      page,
      limit,
      period,
      total,
      total_pages: Math.ceil(total / limit),
    });
  } catch (error) {
    console.error('Leaderboard API error:', error);
    return apiError('Failed to fetch leaderboard', 500);
  }
}

-- `TopologicalSort` moves the first free item into place by a swap, so over
-- `p`, `q`, `s` with both `p` and `q` reading `s` it analyses `s`, then `q`, then `p` —
-- and `q`'s error is the statement's. A stable order would report `p`'s `42703`.
-- @expect-error: the body analysed first in dependency order wins (22P02)
-- @test
WITH RECURSIVE p AS (SELECT nosuchcol FROM s),
  q AS (SELECT k FROM s WHERE k = 'abc'),
  s AS (SELECT 1 AS k)
SELECT 1;

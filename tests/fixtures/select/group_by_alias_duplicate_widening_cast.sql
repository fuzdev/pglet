-- The contrast to `group_by_alias_duplicate_noop_cast.sql`: a **widening** cast is a
-- real coercion, so the two entries sharing the alias genuinely denote different
-- expressions and the `GROUP BY` key is `42702` ambiguous — on both engines.
--
-- This is what pins the no-op-cast strip as *type-directed* rather than a blanket
-- "peel any cast": blanket peeling would make this query wrongly succeed.
-- @setup
CREATE TABLE gbdwc (x INT);
INSERT INTO gbdwc VALUES (1), (2);
-- @test
-- @expect-error: GROUP BY "a" is ambiguous (42702)
SELECT count(*) AS n FROM (SELECT x AS a, x::bigint AS a FROM gbdwc GROUP BY a) q;

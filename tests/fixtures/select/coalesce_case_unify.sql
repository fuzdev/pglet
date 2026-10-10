-- @setup
-- CASE/COALESCE unify branch types: an "unknown" string literal next to a
-- concrete-typed branch is coerced to that type. A parseable literal yields the
-- coerced value (`COALESCE(NULL::int, '7')` → 7); the unparseable case is covered
-- by the companion @expect-error fixture. Same-type branches are untouched.
CREATE TABLE unif (id INT, n INT, label TEXT);
INSERT INTO unif VALUES (1, 10, 'a'), (2, NULL, 'b');

-- @test
SELECT id,
       COALESCE(n, '99') AS coalesced,
       COALESCE(label, 'fallback') AS same_type,
       CASE WHEN id = 1 THEN n ELSE '0' END AS case_branch
FROM unif
ORDER BY id;

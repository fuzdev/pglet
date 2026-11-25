-- A whole row in a grouping query whose `GROUP BY` does not name a primary key is
-- 18.6's `42803`.
-- @no-profile-lock: refused in the select list's rewrite, before any index is chosen
-- @expect-error: the whole row must appear in GROUP BY
-- @test
SELECT to_jsonb(t) FROM (SELECT 1 AS a) t GROUP BY a;

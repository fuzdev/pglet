-- `setval` reaches the same `init_sequence`, so it is 42809 too — the kind check is
-- not `nextval`'s alone.
-- @setup
CREATE TABLE svt_host (a INT);
-- @test
SELECT setval('svt_host', 3);
-- @expect-error: cannot open relation, not a sequence (42809)

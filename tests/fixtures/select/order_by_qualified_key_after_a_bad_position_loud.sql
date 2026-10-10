-- The interleave with a **qualified** key, which reaches the binding gate by the other
-- branch of `correlation::scope_for_key` — the FROM scope rather than the target-list
-- claimants. The ordinal is positionally first, so 18.6 is `42P10`; this reported the key's
-- `42P01`, because the whole key list was resolved before the range check ran over any of
-- it. The reverse spelling (`ORDER BY bogusrel.x, 9`) is `42P01` on both engines, which is
-- what identifies the position rather than the spelling.
-- @setup
CREATE TABLE sk_pos3 (a INT, b INT);
INSERT INTO sk_pos3 VALUES (1, 10);

-- @expect-error: ORDER BY position 9 is not in select list (42P10)
-- @test
SELECT a FROM sk_pos3 ORDER BY 9, bogusrel.x;

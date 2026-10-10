-- Any non-empty argument list resolves the ordered-set `dense_rank`, quantified or not.
-- @setup
CREATE TABLE wna (a INT);
INSERT INTO wna VALUES (1), (2);
-- @test
-- @expect-error: WITHIN GROUP is required for ordered-set aggregate dense_rank (42809)
-- @expect-message
SELECT dense_rank(ALL a, a) OVER (ORDER BY a) FROM wna;

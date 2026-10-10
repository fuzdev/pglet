-- `rank(a)` resolves the hypothetical-set ordered-set aggregate, which needs WITHIN GROUP.
-- @setup
CREATE TABLE wna (a INT);
INSERT INTO wna VALUES (1), (2);
-- @test
-- @expect-error: WITHIN GROUP is required for ordered-set aggregate rank (42809)
-- @expect-message
SELECT rank(a) OVER (ORDER BY a) FROM wna;

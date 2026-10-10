-- A bare `*` over a join qualifies a FROM `unnest`'s columns by its name, so the other
-- side's column of the same name is not a second claimant.
-- @setup
CREATE TABLE un_jn (q INT, y TEXT);
INSERT INTO un_jn VALUES (9, 'a');
-- @test
SELECT * FROM un_jn JOIN unnest(ARRAY[1, 2]) AS u(q) ON true ORDER BY 3;

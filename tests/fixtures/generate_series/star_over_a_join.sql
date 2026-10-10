-- A bare `*` over a join qualifies the series' column by the item's name, so a table
-- column of the same name beside it is not a second claimant.
-- @setup
CREATE TABLE gs_jt (g INT, u INT);
INSERT INTO gs_jt VALUES (7, 8);
-- @test
SELECT * FROM gs_jt, generate_series(1, 2) g ORDER BY 3;

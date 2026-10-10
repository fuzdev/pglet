-- The third route into the DEFAULT cook: `SET DEFAULT` records a general expression
-- and it resolves per inserted row, the same as the column-constraint route's.
-- @setup
CREATE TABLE sd_expr (k INT, v INT);
ALTER TABLE sd_expr ALTER COLUMN v SET DEFAULT 6 * 7;
INSERT INTO sd_expr (k) VALUES (1);
-- @test
SELECT k, v FROM sd_expr;

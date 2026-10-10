-- A range-filtered UPDATE over an indexed key draws its sequence values in row order:
-- `SET n = DEFAULT` over a serial column, rows inserted 3, 1, 2, draws 4, 5, 6 for them in
-- that order.
-- @profile-lock
-- @setup
CREATE TABLE sd (k INT, n SERIAL);
CREATE INDEX sd_k ON sd (k);
INSERT INTO sd (k) VALUES (3), (1), (2);
-- @test
UPDATE sd SET n = DEFAULT WHERE k > 0 RETURNING k, n;

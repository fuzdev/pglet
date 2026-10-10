-- An equality key no `int` value represents exactly (`1.5`) matches no row, and the exact
-- spellings of 1 (`1.0`, `1.00`) match the row 1 — the probe's exact-or-decline, through
-- an UPDATE. The range spelling over `1.5` keeps the rows above it.
-- @profile-lock
-- @setup
CREATE TABLE ip (k INT, v TEXT);
CREATE INDEX ip_k ON ip (k);
INSERT INTO ip VALUES (1, 'a'), (2, 'b'), (NULL, 'n');
-- @test
UPDATE ip SET v = v || 'p' WHERE k = 1.5;
UPDATE ip SET v = v || 'q' WHERE k = 1.0;
UPDATE ip SET v = v || 'r' WHERE k = 1.00;
UPDATE ip SET v = v || 's' WHERE k > 1.5 RETURNING k, v;

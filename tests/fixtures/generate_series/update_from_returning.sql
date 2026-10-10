-- `UPDATE … FROM` a series, its column and its wildcard in `RETURNING`.
-- @setup
CREATE TABLE gs_u (id INT, v INT);
INSERT INTO gs_u VALUES (1, 0), (2, 0), (3, 0);
-- @test
UPDATE gs_u SET v = g * 10 FROM generate_series(1, 2) g WHERE id = g RETURNING id, v, g, g.*;

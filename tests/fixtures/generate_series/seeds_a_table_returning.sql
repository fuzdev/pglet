-- The seeding `INSERT … SELECT` with a `RETURNING`.
-- @setup
CREATE TABLE gs_seed_r (id INT, v INT);
-- @test
INSERT INTO gs_seed_r SELECT g, g * 2 FROM generate_series(10, 12) g RETURNING *;

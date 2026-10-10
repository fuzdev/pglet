-- The seeding idiom with its bound as a parameter, the other value too.
-- @setup
CREATE TABLE gs_ps (id INT, label TEXT);
-- @param $1 int4 4
-- @param $2 text seeded
-- @test
INSERT INTO gs_ps SELECT g, $2 FROM generate_series(1, $1) g RETURNING id, label;

-- A `SERIAL PRIMARY KEY` over a 60-byte table name mints three names, each cut by the one
-- routine under its own label: the sequence's, the primary key's and the column's `NOT
-- NULL`.
-- @setup
CREATE TABLE cac11_tttttttttttttttttttttttttttttttttttttttttttttttttttttt (id SERIAL PRIMARY KEY, v INT);
-- @test
SELECT pg_get_serial_sequence('cac11_tttttttttttttttttttttttttttttttttttttttttttttttttttttt', 'id') AS seq, contype::text AS contype, conname::text AS conname
FROM pg_constraint WHERE conname LIKE 'cac11\_%' ORDER BY contype, conname;

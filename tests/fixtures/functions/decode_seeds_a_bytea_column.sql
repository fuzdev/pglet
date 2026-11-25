-- `decode` as a seed's value for a `bytea` column, with a NULL beside it.
-- @setup
CREATE TABLE dsb (id TEXT, data BYTEA);
-- @test
INSERT INTO dsb VALUES ('a', decode('deadbeef', 'hex')), ('b', NULL) RETURNING id, data, octet_length(data);

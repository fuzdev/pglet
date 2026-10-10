-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- The junk column goes on whichever node the query put at the top, and here that is the
-- `Aggregate`. **Only one minter runs**: the `DISTINCT ON (k)` key mints
-- `pglet_junk_0`, and the trailing `ORDER BY k` then *resolves to* that column
-- (`output_key_identity` finds the Aggregate entry denoting base `k`) rather than
-- minting a second one — so this pins the resolution, not the two-minter trim width.
-- That shape needs a key the first minter's column cannot answer, and is
-- `distinct_on_junk_key_over_an_aggregate_junk_key.sql`.
-- @test
SELECT DISTINCT ON (k) count(*) FROM gj GROUP BY k ORDER BY k DESC;

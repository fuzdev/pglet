-- `integer = boolean` has no operator, and Postgres says so at parse time — for a
-- join too. Here the join produces **zero** row pairs, so the WHERE is never
-- evaluated and the eval-time gate never fires: pglet quietly returned zero rows.
-- The column names across the two relations don't collide, so the static gate can
-- resolve `ka.aid` with certainty; it now runs over multi-relation scopes, not just
-- a single FROM table.
-- @setup
CREATE TABLE ka (aid INT, ak TEXT);
CREATE TABLE kb (bid TEXT, bk INT);
INSERT INTO ka VALUES (1, 'x'), (2, 'y');
INSERT INTO kb VALUES ('1', 10), ('2', 20);
-- @test
-- @expect-error: no integer = boolean operator, even with no joined row pairs
SELECT 1 FROM ka JOIN kb ON ka.aid = kb.bk WHERE ka.aid = true;

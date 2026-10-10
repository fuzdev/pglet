-- The accept polarity at a persisted-expression site: 18.6 takes `ALL` in a column
-- DEFAULT and the stored default behaves as the unquantified call does. The assertion is
-- the backfilled **value** rather than `column_default`, because the two engines render
-- the stored expression differently for reasons outside this fixture's subject
-- (18.6 prints `upper('a'::text)`, pglet `upper('a')`).
-- @setup
CREATE TABLE sq_ad (id INT, c TEXT DEFAULT upper(ALL 'a'));
INSERT INTO sq_ad (id) VALUES (1);
-- @test
SELECT id, c FROM sq_ad;

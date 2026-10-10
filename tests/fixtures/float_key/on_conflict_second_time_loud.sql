-- One statement proposing the same float key twice cannot update the row it inserted.
-- @profile-lock
-- @expect-error: ON CONFLICT DO UPDATE cannot affect a row a second time (21000)
-- @setup
CREATE TABLE fkc (n INT, x FLOAT8 PRIMARY KEY);
INSERT INTO fkc VALUES (1, 0), (2, 'NaN'), (3, 1.5);
-- @test
INSERT INTO fkc VALUES (50, 2.5), (51, 2.5) ON CONFLICT (x) DO UPDATE SET n = excluded.n;

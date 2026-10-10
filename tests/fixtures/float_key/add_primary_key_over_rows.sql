-- `ADD PRIMARY KEY` over a populated `float8` column builds, and then enforces.
-- @profile-lock
-- @setup
CREATE TABLE fkd (n INT, x FLOAT8 NOT NULL);
INSERT INTO fkd VALUES (1, 1.5), (2, 'NaN'), (3, '-Infinity');
ALTER TABLE fkd ADD PRIMARY KEY (x);
INSERT INTO fkd VALUES (4, 2.5);
-- @test
SELECT n, x::text AS shown FROM fkd ORDER BY x;

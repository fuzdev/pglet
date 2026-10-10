-- A reference inside a correlated subquery that resolves at **no** level. Chaining the
-- enclosing relations onto the subquery's scope widens what can resolve; it must not
-- widen what can *silently* resolve. A bare name no level carries is still
-- `42703`, exactly as Postgres reports it — the resolution walk running out of levels
-- leaves the reference untouched, and column resolution rejects it downstream.
-- @setup
CREATE TABLE ck_o (okey INT, ouid UUID);
CREATE TABLE ck_i (ikey INT);
INSERT INTO ck_o VALUES (1, '11111111-1111-1111-1111-111111111111');
INSERT INTO ck_i VALUES (1);
-- @expect-error: bare name carried by no level — 42703
-- @test
SELECT okey FROM ck_o WHERE EXISTS (SELECT 1 FROM ck_i WHERE nosuchcol = '11111111-1111-1111-1111-111111111111');

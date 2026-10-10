-- An equality-probed DELETE over a non-unique key removes that key's rows in row order:
-- inserted 'c', 'a', 'b' under k = 1, `RETURNING` answers c, a, b.
-- @profile-lock
-- @setup
CREATE TABLE pdd (k INT, v TEXT);
CREATE INDEX pdd_k ON pdd (k);
INSERT INTO pdd VALUES (2, 'm'), (1, 'c'), (1, 'a'), (3, 'x'), (1, 'b');
-- @test
DELETE FROM pdd WHERE k = 1 RETURNING k, v;

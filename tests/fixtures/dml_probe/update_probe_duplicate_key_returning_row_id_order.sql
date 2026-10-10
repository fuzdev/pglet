-- An equality-probed UPDATE over a non-unique key writes that key's rows in row order:
-- inserted 'c', 'a', 'b' under k = 1, `RETURNING` answers c, a, b.
-- @profile-lock
-- @setup
CREATE TABLE pd (k INT, v TEXT);
CREATE INDEX pd_k ON pd (k);
INSERT INTO pd VALUES (1, 'c'), (1, 'a'), (2, 'x'), (1, 'b');
-- @test
UPDATE pd SET v = v || '!' WHERE k = 1 RETURNING k, v;

-- `UPDATE … FROM` assembles `target ++ source` and only splits the source half off after
-- the assignments. The `Failing row` must describe the **target** alone, so the not-null
-- gate runs after that split — checking the joined row would put the source's columns in
-- the detail.
-- @setup
CREATE TABLE y3 (a INT NOT NULL, b TEXT);
CREATE TABLE y3s (x INT, y TEXT);
INSERT INTO y3 VALUES (1, 't');
INSERT INTO y3s VALUES (1, 'srcval');

-- @test
-- @expect-error: the assignment nulls a NOT NULL column
-- @expect-message
UPDATE y3 SET a = NULL FROM y3s WHERE y3.a = y3s.x;

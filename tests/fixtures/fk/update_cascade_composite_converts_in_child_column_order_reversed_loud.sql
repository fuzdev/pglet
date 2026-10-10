-- A composite `ON UPDATE CASCADE` converts the referencing columns in the child table's
-- column order: `b` precedes `a` in the table, so `b`'s `22003` is raised before `a`'s
-- `22001`, though the key lists `a` first.
-- `update_cascade_composite_converts_in_child_column_order_loud.sql` is the mirror.
-- @setup
CREATE TABLE p (x varchar(10), y int, PRIMARY KEY (x, y));
CREATE TABLE c (id int PRIMARY KEY, b int2, a varchar(2), FOREIGN KEY (a, b) REFERENCES p (x, y) ON UPDATE CASCADE);
INSERT INTO p VALUES ('b', 2);
INSERT INTO c VALUES (1, 2, 'b');
-- @test
-- @profile-lock
-- @expect-error: smallint out of range
UPDATE p SET x = 'abcdef', y = 40000;

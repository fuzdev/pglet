-- A composite `ON UPDATE CASCADE` converts the referencing columns in the child table's
-- column order, not the key's: `a` precedes `b` in the table, so `a`'s `22001` is raised
-- before `b`'s `22003`, though the key lists `b` first.
-- `update_cascade_composite_converts_in_child_column_order_reversed_loud.sql` is the mirror.
-- @setup
CREATE TABLE p (x varchar(10), y int, PRIMARY KEY (x, y));
CREATE TABLE c (id int PRIMARY KEY, a varchar(2), b int2, FOREIGN KEY (b, a) REFERENCES p (y, x) ON UPDATE CASCADE);
INSERT INTO p VALUES ('b', 2);
INSERT INTO c VALUES (1, 'b', 2);
-- @test
-- @profile-lock
-- @expect-error: value too long for type character varying(2)
UPDATE p SET x = 'abcdef', y = 40000;

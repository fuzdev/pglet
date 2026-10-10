-- An action's conversion is refused when the action's own turn in the queue comes, not
-- when the moved row queues it: the `RESTRICT` key on `r`, created first, fires first and
-- refuses `23001` before the `varchar(3)` cascade into `v` is reached.
-- @setup
CREATE TABLE p (k varchar(10) PRIMARY KEY);
CREATE TABLE r (id int PRIMARY KEY, pk varchar(10) REFERENCES p ON UPDATE RESTRICT);
CREATE TABLE v (id int PRIMARY KEY, pk varchar(3) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES ('a');
INSERT INTO r VALUES (1, 'a');
INSERT INTO v VALUES (1, 'a');
-- @test
-- @profile-lock
-- @expect-error: violates RESTRICT setting of foreign key constraint "r_pk_fkey"
UPDATE p SET k = 'abcdef';

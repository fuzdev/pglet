-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- `pb.x` is a grouping column the select list drops. Postgres carries it as a junk
-- target-list entry and sorts by it; pglet's Sort sits above the Aggregate, so the
-- Aggregate emits it as a junk column and a `JunkFilter` trims it back off. The two
-- relations' `x` values run in **opposite** directions precisely so a bare-name
-- substitution is observable: answering `pb.x` with the projected `pa.x` sorts the
-- other way (10, 20, 30) with no error at all.
-- @test
SELECT pa.x FROM pa JOIN pb ON pa.id = pb.id GROUP BY pa.x, pb.x ORDER BY pb.x;

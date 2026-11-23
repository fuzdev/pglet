-- A select-list call under an `ORDER BY` over input columns is evaluated in the sorted
-- order, as 18.6 evaluates a volatile select-list item after the sort: one hold of key 1,
-- so the first row in output order (`a = 2`) gives it back (`true`) and the second answers
-- `false`. Evaluated in scan order, the two would land the other way round.
-- @setup
CREATE TABLE aus (a INT);
INSERT INTO aus VALUES (1), (2);
-- @test
SELECT pg_advisory_lock(1);
SELECT pg_advisory_unlock(1) AS released FROM aus ORDER BY a DESC;

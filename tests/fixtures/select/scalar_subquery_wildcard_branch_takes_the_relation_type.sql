-- A branch whose subquery projects a **wildcard** has no target-list entry to read, and
-- its type is the relation's own — read from the catalog rather than guessed — so the
-- branch set unifies to `numeric` and every row is sent at that type. Left unified, the
-- row that takes the `integer` branch sent four bytes under a `numeric` descriptor,
-- which the binary leg here is what sees.
-- @setup
CREATE TABLE sswn (c NUMERIC(6,2));
INSERT INTO sswn VALUES (1.50);
CREATE TABLE sswk (id INT);
INSERT INTO sswk VALUES (1), (2);
-- @test
SELECT k.id, COALESCE((SELECT * FROM sswn WHERE c > k.id), 2) AS v FROM sswk k ORDER BY 1;

-- The strongest form of the restore control: a column actually **named** `float`. The
-- speculative type-name read must put the cursor back at each spelling below — bare,
-- under a qualifier, inside an aggregate's argument list, and in the `WHERE`,
-- `GROUP BY` and `ORDER BY` clauses — or the column becomes unreachable. `float` is a
-- `ColId` on 18.6 and a legal column name here, so `CREATE TABLE` accepting it is part
-- of the assertion.
-- @setup
CREATE TABLE fnc (float int, x int);
INSERT INTO fnc VALUES (1, 10), (2, 20), (2, 30);
-- @test
SELECT float, fnc.float AS qualified, count(float) AS seen, sum(x) AS total
  FROM fnc
 WHERE float > 1
 GROUP BY float, fnc.float
 ORDER BY float;

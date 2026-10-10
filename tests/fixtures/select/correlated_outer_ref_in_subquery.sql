-- The correlated outer reference inside an `IN (SELECT …)` sub-plan. `Expr::InSubquery`
-- carries its plan as a walker leaf just like `Exists`, so the enclosing coercion pass
-- never reached the comparison and the literal stayed text (`42883`, where Postgres
-- returns the row). Both the `IN` and the `NOT IN` spelling are covered — the negation
-- rides the same node, so a descent that missed one would miss both.
-- @setup
CREATE TABLE ci_o (okey INT, ouid UUID);
CREATE TABLE ci_i (ikey INT, itag TEXT);
INSERT INTO ci_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO ci_i VALUES (1, 'a');
-- @test
SELECT okey,
       okey IN (SELECT ikey FROM ci_i WHERE ci_o.ouid = '11111111-1111-1111-1111-111111111111') AS in_hit,
       okey NOT IN (SELECT ikey FROM ci_i WHERE ci_o.ouid = '11111111-1111-1111-1111-111111111111') AS not_in_hit
  FROM ci_o ORDER BY okey;

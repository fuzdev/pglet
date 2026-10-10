-- **An error-order control.** 18.6 raises the assignment cast in parse analysis
-- (`transformAssignedExpr`) and the identity refusal in the **rewriter**
-- (`rewriteTargetListIU`), which runs after all of parse analysis — so the `42804` wins
-- no matter which column is written first. Measured in both orders on 18.6; this file
-- pins the arrangement that discriminates, with the identity column *ahead* of the
-- ill-typed one.
-- @setup
CREATE TABLE acid (id INT GENERATED ALWAYS AS IDENTITY, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
INSERT INTO acid (id, b) VALUES (5, 1);

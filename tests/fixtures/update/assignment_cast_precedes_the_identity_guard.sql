-- **An error-order control**, and the one that moved a check. 18.6 raises the assignment
-- cast in parse analysis and the identity refusal in the rewriter, so the `42804` wins
-- even though the identity column is written first. pglet ran its identity guard at the
-- **top** of the UPDATE lowering, ahead of every clause, so it beat this — and beat the
-- `WHERE`'s and the `RETURNING`'s errors too, both of which 18.6 also reports over it.
-- The guard now runs last, after the whole `SET` list.
--
-- Its neighbour `ddl/identity_always_update_loud.sql` holds the other direction:
-- with nothing else wrong, the identity refusal is still what both engines answer.
-- @setup
CREATE TABLE acuid (id INT GENERATED ALWAYS AS IDENTITY, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
UPDATE acuid SET id = 5, b = 1;

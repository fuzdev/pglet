-- A partial-index predicate's comparison operands are type-checked at CREATE
-- INDEX (`42883`), not deferred to the first write that maintains the index.
-- @setup
CREATE TABLE idx_cmp_pred (id INT, name TEXT);
-- @test
-- @expect-error: no text = integer operator
CREATE INDEX idx_cmp_pred_p ON idx_cmp_pred (id) WHERE name = 5;

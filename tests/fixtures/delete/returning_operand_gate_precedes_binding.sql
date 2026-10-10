-- The mirror of `delete/returning_binding_precedes_operand_gate.sql`: the same two faults,
-- written the other way round, so the operator is leftmost and `42883` wins. This half is
-- green **before and after** the binding check landed — it exists to reject a wrong
-- implementation of it (a binding pass run ahead of the operand gate would answer `42703`
-- here), not to lock a behaviour change. Read the two files together or neither.
-- @setup
CREATE TABLE rog_t (id INT, b BIGINT);
INSERT INTO rog_t VALUES (1, 111);

-- @test
-- @expect-error: the leftmost fault is the operator (42883), not the unbindable column (42703)
DELETE FROM rog_t WHERE id = 999 RETURNING b LIKE 'x' AND bogus > 0;

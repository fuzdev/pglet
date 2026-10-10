-- The INTERNAL-edge shape: a hint and no detail, because CASCADE does not help.
-- @setup
CREATE TABLE t_id (a INT GENERATED ALWAYS AS IDENTITY);

-- @test
-- @expect-error: the identity column requires its sequence
-- @expect-message
DROP SEQUENCE t_id_a_seq;

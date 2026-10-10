-- Precedence: the target is resolved before the owner, so a statement wrong in both
-- places reports the target (42P01).
-- @test
ALTER SEQUENCE no_such_sequence_prec OWNED BY also_no_such_table.a;
-- @expect-error: relation does not exist (42P01)

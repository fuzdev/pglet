-- The interval CHECK's render probe stands in for **no** 18.6 error, so it must lose
-- to every gate that stands in for one — here a constraint index whose name the
-- relation namespace already holds (`42P07`).
--
-- The CHECK is written **first**, which is the arrangement that used to diverge: the
-- probe rode inside `validate_check_exprs`, and `execute_create_table` runs its whole
-- CHECK loop before `mint_constraint_index`, so `0A000` pre-empted the `42P07` in
-- **both** source orders — this one and its reverse. That is why there is no reverse
-- fixture beside it: for this kind the order was never the variable, the position of
-- the probe was, and one arrangement pins it. Measured on both engines before and
-- after the move.
-- @expect-error: the constraint index's taken relation name, ahead of the interval CHECK (42P07)
-- @setup
CREATE TABLE iv_ixn_taken (z INT);
-- @test
CREATE TABLE iv_ixn (
  t TIMESTAMPTZ,
  n INT,
  CHECK (t > now() - interval '1 day'),
  CONSTRAINT iv_ixn_taken UNIQUE (n)
);

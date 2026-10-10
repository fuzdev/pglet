-- **An error-order control.** The identity refusal is a *rewriter* check on 18.6, which
-- runs after all of parse analysis — so it loses to the `WHERE`'s and the `RETURNING`'s
-- errors and to the `SET` list's own `42804`, all of which
-- `assignment_cast_precedes_the_identity_guard.sql` and its neighbours pin.
--
-- **After every clause is still not after every Postgres error**, and this is the pair
-- that says so: an unresolvable `FROM` relation is `42P01` in 18.6's range-table
-- resolution, ahead of parse analysis entirely, while pglet raises it at *execution* —
-- downstream of every plan-time verdict. So the guard has to share the same decline the
-- assignment gate takes, and moving it to the end of the lowering does not reach this
-- statement either way (measured `428C9` here both before and after that move).
-- @setup
CREATE TABLE acig (id INT GENERATED ALWAYS AS IDENTITY, v INT);

-- @expect-error: relation "acig_nosuch" does not exist (42P01)
-- @test
UPDATE acig SET id = 5 FROM acig_nosuch;

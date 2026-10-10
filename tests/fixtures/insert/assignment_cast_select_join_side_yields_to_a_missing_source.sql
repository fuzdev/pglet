-- **An error-order control — do not trim it.** Here the source column is perfectly
-- typed — `acjm_ok.n` is a real `text` column of a real table — and the statement is
-- still `42P01` on 18.6, because the *other* side of the join does not exist.
--
-- That is what a per-expression "this walker cannot type it" skip cannot see, and why
-- the decline is a property of the whole source plan: the missing relation contributes
-- nothing to the expression whose type the gate is about to refuse. The same argument
-- `update/assignment_cast_set_yields_to_a_missing_source.sql` makes for a right-hand
-- side of `1`.
-- @setup
CREATE TABLE acjm (id INT, i INT);
CREATE TABLE acjm_ok (n TEXT);

-- @expect-error: relation "acjm_nosuch" does not exist (42P01)
-- @test
INSERT INTO acjm (i) SELECT a.n FROM acjm_ok a JOIN acjm_nosuch b ON TRUE;

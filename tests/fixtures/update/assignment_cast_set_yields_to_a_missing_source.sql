-- **An error-order control — do not trim it.** `transformFromClause` runs before
-- `transformUpdateTargetList`, so a `FROM` relation that does not exist is `42P01` and
-- the `SET` list is never coerced. pglet raises that `42P01` at **execution**, so the
-- plan-time gate must decline for the whole statement when any relation the statement
-- names is unresolvable — a per-expression skip is not enough, because this right-hand
-- side is a perfectly typed `1` whose pair with `BOOL` is genuinely illegal. Three
-- siblings pin the rest of that rule: `…_in_the_where.sql` and `…_in_returning.sql` for
-- the other two clauses `transformUpdateStmt` runs ahead of the target list, and
-- `assignment_cast_set_yields_to_a_missing_derived_source.sql` for the case a mere "did
-- the source register a relation" test cannot see.
-- @setup
CREATE TABLE acums (id INT, b BOOL);

-- @expect-error: relation "acums_nosuch" does not exist (42P01)
-- @test
UPDATE acums SET b = 1 FROM acums_nosuch;

-- `unnest(<a non-array>)` is `42883` at plan time, where signature resolution sits.
--
-- The headline of the class: over an **empty** table this used to answer `(0 rows)`
-- under a full `RowDescription`, and the two declared-type walkers disagreed about what
-- to put in it — the executor walk answered the argument's own type (23 here), the
-- planner walk nothing and its totality boundary substituted 25 — for a statement 18.6
-- refuses outright. Neither answer was right; the close is the refusal, not a choice
-- between them.
--
-- Row-independent by construction: the gate reads the argument's **declared** type in
-- `check_expr_semantics`'s one bottom-up walk, so the verdict does not depend on a row
-- reaching the node. `unnest_projection_non_array_column_with_rows_loud.sql` is the
-- other end of that axis and `unnest_projection_int_declared_type.sql` the array pole,
-- which must keep answering.
-- @setup
CREATE TABLE unnest_na (i INT, tx TEXT, arr INT[]);
-- @test
-- @expect-error: no unnest(integer)
SELECT unnest(i) FROM unnest_na;

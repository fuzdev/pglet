-- `RETURNING *` and a qualified `u.u` over a **FROM-clause `unnest(…)`** source.
--
-- Two things have to line up for this, and they live on opposite sides of the engine.
-- The planner registers the relation under the alias and names its single column after
-- that alias too (Postgres's `chooseScalarFunctionAlias`), so the wildcard expands to
-- `u.u`. The **executor** has to key its own namespace on the same name: it derived the
-- source's name from the plan node and had no `unnest` arm, so the relation went in as
-- `"unknown"` and every qualified reference to it was `42P01` — on `WHERE u.u = 'x'`
-- (which 18.6 answers) as much as on `RETURNING u.u`. A build with only the planner half
-- reddens here, at the qualified item.
--
-- The accepting/ambiguous polarity of the *bare* spelling is
-- `returning_from_unnest_source_column_ambiguous.sql` and
-- `returning_from_unnest_source_unambiguous_name_answers.sql`.
-- @setup
CREATE TABLE ruw_t (id INT, v TEXT);
INSERT INTO ruw_t VALUES (1, 'T');

-- @test
UPDATE ruw_t SET id = 1 FROM unnest(ARRAY['x', 'y']) AS u WHERE u.u = 'x' RETURNING *, u.u AS qualified;

-- **18.6 renders a `DEFAULT`'s coerced *value*; pglet renders the literal *as written*.**
-- `pg_get_expr` prints the `Const` that parse analysis produced, through the type's
-- output function — never the source text. For a type pglet renders **verbatim**, which
-- every type in this fixture is, the two engines can therefore agree only at literals
-- whose text is already what that output function produces, and every literal below is
-- spelled that way on purpose: note the **offset-bearing** `'…+00'` for `timestamptz`,
-- whose `timestamptz_out` appends the `+00` a bare spelling omits.
--
-- The verbatim scope is the whole of it: `deparse::render_catalog_string_const` folds
-- for `bool`, `int2`, `int4`, `int8` and `numeric`, reproducing those output functions,
-- so a `bool DEFAULT 'yes'` or an `int DEFAULT '007'` agrees from a source text that is
-- nothing like the output form. Those are
-- `catalog/column_default_string_literal_folds_through_its_output_function.sql`'s.
--
-- Any other accepted spelling of a verbatim-rendered literal diverges, and that is the
-- rule rather than an exception to it. Measured:
-- `timestamptz(3) DEFAULT '2020-01-02 03:04:05.6789'` reads
-- `'…+00'::timestamp with time zone` on 18.6 against pglet's offsetless string, and
-- `timestamp(3) DEFAULT '2020-01-02T03:04:05.6789'` reads the space-separated form there
-- against the `T` here. `interval` diverges on **two** axes, because `coerce_type` passes
-- the typmod to the input function for `INTERVALOID` alone (`parse_coerce.c`): 18.6
-- records `'00:00:01.235'::interval(3)` where pglet records `'1.23456 secs'::interval`,
-- differing in the value and in the label — so an `interval(3)` column cannot join this
-- fixture at any spelling of its literal, its label diverging even when the literal is
-- already in output form. The label divergence is the **modifier**'s, not the type's: a
-- bare `interval` column carries no modifier for 18.6 to print, and `interval DEFAULT
-- '00:00:01.235'` and `DEFAULT '1 day 02:03:04'` each read `'…'::interval` on both
-- engines, measured.
-- All of those are `EDGES.md` row 123's and pre-existing; the *stored* value rounds
-- either way, which `ddl/precision_default_values_insert.sql` pins for `timestamp` and
-- `time` and `ddl/precision_add_column_backfill.sql` for `timestamp` and `interval`.
-- @setup
CREATE TABLE tdl (a timestamp(3) DEFAULT '2020-01-02 03:04:05.6789',
                  b timestamptz(3) DEFAULT '2020-01-02 03:04:05.6789+00',
                  c time(3) DEFAULT '03:04:05.6789',
                  d varchar(3) DEFAULT 'ab');
-- @no-declared-types: `column_name` is `sql_identifier` (base `name`, OID 19) and
--   `column_default` is `character_data` (base `varchar`, 1043) on 18.6, where pglet
--   declares `text` (25) for both — measured by removing this waiver. Pre-existing
--   catalog-type modelling, not this fixture's subject.
-- @test
SELECT column_name, column_default
FROM information_schema.columns
WHERE table_name = 'tdl'
ORDER BY ordinal_position;

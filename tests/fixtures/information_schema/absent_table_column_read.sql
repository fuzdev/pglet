-- **A false-rejection control — do not trim it.** pglet models a strict subset of 18.6's
-- columns for 7 of the 12 catalog relations it has (`information_schema.columns` is 8 of
-- 44), so a binding gate reading that list as complete calls `udt_name` a column the view
-- does not have and answers `42703` — where both engines return no rows.
--
-- A catalog view is therefore registered **non-exhaustively**: it answers its own
-- qualifier, and a name absent from its list is *unknown* rather than absent. The price is
-- recorded and is the safe direction: `SELECT nosuchcol_at_all FROM
-- information_schema.columns` is `42703` on 18.6 and `(0 rows)` here.
--
-- The empty result is the point, not an accident of the seed — no table is named
-- `nosuchtable_xyz`, on either engine.
-- @setup
CREATE TABLE atc_anchor (id INT);

-- @no-declared-types: `udt_name` is modelled now, but 18.6 sends `name` (OID 19) for it
-- and pglet sends `text` (25). The domain the column is declared as
-- (`information_schema.sql_identifier`) never reaches the wire — 18.6 sends the domain's
-- **base** type — so what is missing here is a declared type for the catalog view's
-- column, not a hard-codable OID. Outside this fixture's subject, which is that a read
-- over an absent table is `(0 rows)` and not `42703`.
-- @test
SELECT udt_name FROM information_schema.columns WHERE table_name = 'nosuchtable_xyz';

-- The accept half of the target-list ambiguity rule at a level's **own** lowering, and the
-- spelling that exercises the *identity normalization* rather than the rule: wildcard
-- expansion emits a **qualified** reference, so the star's `c` and the written `c` differ
-- in spelling while denoting one column. `findTargetlistEntrySQL92` refuses two matching
-- entries only when they are not `equal()` — its own worked example is *"we allow SELECT a,
-- a FROM table ORDER BY a"* — so both engines answer.
--
-- It must stay green while `select/order_by_ambiguous_output_alias_loud.sql` stays red, or
-- the claimant set has degenerated into a count by name, which refuses ordinary SQL. The
-- bare-column binding that makes the two entries compare equal is
-- `correlation::entry_identity`, shared with the correlated re-walk — the nested twin of
-- this fixture is `select/nested_sort_key_duplicate_output_entries_resolve.sql`.
--
-- `WHERE false` keeps the key away from the **eval-time** resolver, which still counts by
-- name over a row it is handed: a pre-existing, row-count-dependent over-rejection
-- recorded in `docs/compatibility.md`'s SELECT row and not this gate's.
-- @setup
CREATE TABLE sk_dup (c INT, x TEXT);
INSERT INTO sk_dup VALUES (1, 'a'), (2, 'b');

-- @test
SELECT *, c FROM sk_dup WHERE false ORDER BY c;

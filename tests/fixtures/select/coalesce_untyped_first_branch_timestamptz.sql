-- The un-rewritten-branch split (mechanism:
-- `coalesce_untyped_first_branch_declared_type.sql`) reaching a **value** and not only
-- an OID.
--
-- `timestamptz`'s text encoding is column-type-aware: only the declared OID carries the
-- tz/no-tz bit, so a `timestamptz` value declared anything else renders without its
-- `+00` offset and a text-protocol driver reparses the instant in its own local zone.
-- The executor walk's declaration is what that encoder reads, so a `COALESCE(NULL,
-- date_trunc('day', ts))` — both branches invisible to `concrete_branch_type`, hence no
-- unifying `Cast` — sent `2024-01-15 00:00:00` where 18.6 sends
-- `2024-01-15 00:00:00+00`, over a `Describe` that correctly said `timestamptz`. A
-- corrupted instant, not a mislabelled column, and it is the reason this family is a
-- `fix:` and not a cosmetic one.
--
-- The binary leg was **green on the defect**: `timestamp` and `timestamptz` share the
-- same 8-byte encoding, and `Describe` declared the right one anyway. Only the live
-- text comparison sees it, which is why this fixture is an ordinary `@test` (both legs)
-- rather than a `@text` snapshot — the value must match Postgres live, and the declared
-- type must match on both legs.
--
-- `typed_first` is the polarity control: the same two branches in the other order,
-- where the executor walk already read the `date_trunc`.
-- @setup
CREATE TABLE tsc (id INT, ts TIMESTAMPTZ);
INSERT INTO tsc VALUES (1, '2024-01-15T10:30:00Z');

-- @test
SELECT COALESCE(NULL, date_trunc('day', ts)) AS untyped_first, -- timestamptz, with +00
       COALESCE(date_trunc('day', ts), NULL) AS typed_first    -- timestamptz — control
FROM tsc;

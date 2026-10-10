//! Engine integration tests: a parameter whose type the client declared at `Parse`
//! has that type for parse analysis (`parser::parse_single_with_param_types`).
//!
//! The differential fixtures (`params/declared_*`) hold these shapes over the wire, and
//! only through `Describe`'s descriptor — the planner walk — because a bound statement
//! has no simple-query leg. These tests hold the **other** walk too: the executor's
//! result columns are what the simple-query `RowDescription` and the column-type-aware
//! text encoder read, so the two are asserted equal to one expected type per column.

use crate::common;

#[cfg(test)]
mod declared {
    use super::common::*;
    use pglet_sql::executor::{EvalEnv, coerce_value, execute_with_storage};
    use pglet_sql::parser::{parse_single, parse_single_with_param_types};
    use pglet_sql::types::{TypeOid, Value};

    /// Plan `sql` with `declared` parameter types, execute it with `params` (each a text
    /// spelling decoded as its parameter's type, as a text-format `Bind` decodes it), and
    /// return the planner walk's column types, the executor walk's column types, and the
    /// rows.
    fn run_declared(
        db: &mut TestDb,
        sql: &str,
        declared: &[Option<TypeOid>],
        params: &[(&str, TypeOid)],
    ) -> (Vec<TypeOid>, Vec<TypeOid>, Vec<Vec<Value>>) {
        let plan = parse_single_with_param_types(sql, &db.schema, declared)
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .expect("a statement");
        let planner: Vec<TypeOid> = plan
            .result_shape(&db.schema)
            .columns()
            .iter()
            .map(|c| c.type_oid)
            .collect();
        let values: Vec<Option<Value>> = params
            .iter()
            .map(|(text, oid)| Some(coerce_value(Value::Text((*text).to_string()), *oid).unwrap()))
            .collect();
        let result = execute_with_storage(
            &plan,
            &mut db.schema,
            &mut db.storage,
            EvalEnv::with_params(&values),
            &mut db.sequence_session,
        )
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
        let executor = result.columns().iter().map(|c| c.type_oid).collect();
        (planner, executor, result.rows)
    }

    fn numeric(text: &str) -> Value {
        coerce_value(Value::Text(text.to_string()), TypeOid::Numeric).unwrap()
    }

    #[test]
    fn a_declared_parameter_types_both_walks_and_keeps_its_value() {
        let mut db = TestDb::new();
        let n = Some(TypeOid::Numeric);
        // (statement, the column types 18.6 describes, the row it answers) — each measured
        // on 18.6 over a raw extended-protocol client with `$1` declared `numeric` and bound
        // `1.5` as text.
        let cases: [(&str, Vec<TypeOid>, Vec<Value>); 6] = [
            ("SELECT $1", vec![TypeOid::Numeric], vec![numeric("1.5")]),
            (
                "SELECT coalesce($1, 1)",
                vec![TypeOid::Numeric],
                vec![numeric("1.5")],
            ),
            (
                "SELECT CASE WHEN true THEN $1 ELSE 1 END",
                vec![TypeOid::Numeric],
                vec![numeric("1.5")],
            ),
            (
                "SELECT greatest($1, 1), nullif($1, 0)",
                vec![TypeOid::Numeric, TypeOid::Numeric],
                vec![numeric("1.5"), numeric("1.5")],
            ),
            (
                "SELECT $1 + 1",
                vec![TypeOid::Numeric],
                vec![numeric("2.5")],
            ),
            (
                "SELECT round($1)",
                vec![TypeOid::Numeric],
                vec![numeric("2")],
            ),
        ];
        for (sql, types, row) in cases {
            let (planner, executor, rows) =
                run_declared(&mut db, sql, &[n], &[("1.5", TypeOid::Numeric)]);
            assert_eq!(planner, types, "planner walk over {sql}");
            assert_eq!(executor, types, "executor walk over {sql}");
            assert_eq!(rows, vec![row], "rows of {sql}");
        }

        // `ARRAY[$1]` declared `int4` is `int4[]` on both walks, and a declared
        // `timestamptz` under `date_trunc` stays `timestamptz` — the type the text encoder
        // renders the `+00` offset by.
        let (planner, executor, _) = run_declared(
            &mut db,
            "SELECT ARRAY[$1]",
            &[Some(TypeOid::Int4)],
            &[("5", TypeOid::Int4)],
        );
        assert_eq!(
            (planner, executor),
            (vec![TypeOid::Int4Array], vec![TypeOid::Int4Array])
        );
        let (planner, executor, _) = run_declared(
            &mut db,
            "SELECT date_trunc('day', $1)",
            &[Some(TypeOid::Timestamptz)],
            &[("2000-01-01 12:00:00+05", TypeOid::Timestamptz)],
        );
        assert_eq!(
            (planner, executor),
            (vec![TypeOid::Timestamptz], vec![TypeOid::Timestamptz])
        );
    }

    #[test]
    fn an_undeclared_parameter_keeps_its_plan() {
        // The polarity control: with no declared types the parser reads a bare `$N`, the
        // plan is the one `parse_single` builds, and `coalesce($1, 1)` still unifies the
        // parameter to its sibling's `integer`. A `None` entry is the same as no entry.
        let db = TestDb::new();
        let sql = "SELECT coalesce($1, 1), $2 + 0";
        let bare = parse_single(sql, &db.schema).unwrap().unwrap();
        let with_none = parse_single_with_param_types(sql, &db.schema, &[None, None])
            .unwrap()
            .unwrap();
        assert_eq!(bare, with_none);
        let with_one = parse_single_with_param_types(sql, &db.schema, &[None, Some(TypeOid::Int8)])
            .unwrap()
            .unwrap();
        assert_ne!(bare, with_one, "a declared `$2` must reach the plan");
        let types: Vec<TypeOid> = bare
            .result_shape(&db.schema)
            .columns()
            .iter()
            .map(|c| c.type_oid)
            .collect();
        assert_eq!(types[0], TypeOid::Int4);
    }

    #[test]
    fn a_declared_parameter_decides_which_row_a_predicate_matches() {
        // It writes: `n = coalesce($1, 0)` with `$1` declared `numeric` and bound `1.5`
        // removes the `1.5` row. Converted to the literal's `integer`, the parameter was `2`
        // and the `DELETE` removed the other row.
        let mut db = TestDb::new();
        db.run("CREATE TABLE dp (n NUMERIC)");
        db.run("INSERT INTO dp VALUES (1.5), (2)");
        let (_, _, rows) = run_declared(
            &mut db,
            "DELETE FROM dp WHERE n = coalesce($1, 0) RETURNING n",
            &[Some(TypeOid::Numeric)],
            &[("1.5", TypeOid::Numeric)],
        );
        assert_eq!(rows, vec![vec![numeric("1.5")]]);
        let left = db.run("SELECT n FROM dp");
        assert_eq!(left.rows, vec![vec![numeric("2")]]);
    }

    #[test]
    fn a_declared_parameter_without_a_common_type_is_refused() {
        // 18.6's `select_common_type` refuses the pair: `42804 COALESCE types boolean and
        // integer cannot be matched`, where the old conversion answered `1`.
        let db = TestDb::new();
        for (oid, name) in [
            (TypeOid::Bool, "boolean"),
            (TypeOid::Date, "date"),
            (TypeOid::Text, "text"),
        ] {
            let err =
                parse_single_with_param_types("SELECT coalesce($1, 1)", &db.schema, &[Some(oid)])
                    .expect_err(name);
            assert_eq!(err.sqlstate(), "42804", "{name}: {err}");
            assert!(
                err.to_string().contains(&format!(
                    "COALESCE types {name} and integer cannot be matched"
                )),
                "{name}: {err}"
            );
        }
    }
}

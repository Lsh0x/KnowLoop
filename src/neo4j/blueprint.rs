//! Neo4j Blueprint operations
//!
//! Implements all CRUD and relation operations for the Blueprint Engine
//! on the Neo4j graph.

use super::client::Neo4jClient;
use crate::blueprint::{
    BlueprintNode, BlueprintRelationType, BlueprintResponse, BlueprintStatus,
    CreateBlueprintRequest, ListBlueprintsQuery, UpdateBlueprintRequest,
};
use anyhow::{Context, Result};
use neo4rs::query;
use uuid::Uuid;

impl Neo4jClient {
    // ========================================================================
    // Conversion helpers
    // ========================================================================

    /// Convert a Neo4j node to a [`BlueprintNode`].
    pub(crate) fn node_to_blueprint(node: &neo4rs::Node) -> Result<BlueprintNode> {
        let tags: Vec<String> = node.get("tags").unwrap_or_default();
        let stack: Vec<String> = node.get("stack").unwrap_or_default();

        Ok(BlueprintNode {
            id: node.get::<String>("id")?.parse()?,
            slug: node.get("slug").unwrap_or_default(),
            name: node.get("name")?,
            description: node.get("description").unwrap_or_default(),
            scope: node
                .get::<String>("scope")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            category: node
                .get::<String>("category")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            difficulty: node
                .get::<String>("difficulty")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            estimated_time: node.get("estimated_time").unwrap_or_default(),
            stack,
            tier1: node.get("tier1").unwrap_or_default(),
            tier2: node.get("tier2").unwrap_or_default(),
            tier3_content: node.get("tier3_content").unwrap_or_default(),
            source_file: node.get("source_file").unwrap_or_default(),
            content_hash: node.get("content_hash").unwrap_or_default(),
            status: node
                .get::<String>("status")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            version: node.get("version").unwrap_or(1),
            tags,
            usage_count: node.get("usage_count").unwrap_or(0),
            created_at: node
                .get::<String>("created_at")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(chrono::Utc::now),
            updated_at: node
                .get::<String>("updated_at")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(chrono::Utc::now),
        })
    }

    // ========================================================================
    // CRUD operations
    // ========================================================================

    /// Create a new Blueprint node from a request DTO.
    pub async fn create_blueprint(&self, req: &CreateBlueprintRequest) -> Result<BlueprintNode> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let scope = req.scope.unwrap_or_default();
        let category = req.category.unwrap_or_default();
        let difficulty = req.difficulty.unwrap_or_default();

        let q = query(
            r#"
            CREATE (b:Blueprint {
                id: $id,
                slug: $slug,
                name: $name,
                description: $description,
                scope: $scope,
                category: $category,
                difficulty: $difficulty,
                estimated_time: $estimated_time,
                stack: $stack,
                tier1: $tier1,
                tier2: $tier2,
                tier3_content: $tier3_content,
                source_file: $source_file,
                content_hash: $content_hash,
                status: $status,
                version: 1,
                tags: $tags,
                usage_count: 0,
                created_at: $created_at,
                updated_at: $updated_at
            })
            RETURN b
            "#,
        )
        .param("id", id.to_string())
        .param("slug", req.slug.clone())
        .param("name", req.name.clone())
        .param("description", req.description.clone().unwrap_or_default())
        .param("scope", scope.to_string())
        .param("category", category.to_string())
        .param("difficulty", difficulty.to_string())
        .param(
            "estimated_time",
            req.estimated_time.clone().unwrap_or_default(),
        )
        .param("stack", req.stack.clone().unwrap_or_default())
        .param("tier1", req.tier1.clone().unwrap_or_default())
        .param("tier2", req.tier2.clone().unwrap_or_default())
        .param(
            "tier3_content",
            req.tier3_content.clone().unwrap_or_default(),
        )
        .param("source_file", req.source_file.clone().unwrap_or_default())
        .param("content_hash", req.content_hash.clone().unwrap_or_default())
        .param("status", BlueprintStatus::Active.to_string())
        .param("tags", req.tags.clone().unwrap_or_default())
        .param("created_at", now.to_rfc3339())
        .param("updated_at", now.to_rfc3339());

        let mut result = self
            .graph
            .execute(q)
            .await
            .context("Failed to create Blueprint node")?;

        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            Self::node_to_blueprint(&node)
        } else {
            anyhow::bail!("CREATE did not return blueprint node")
        }
    }

    /// Get a blueprint by UUID string.
    pub async fn get_blueprint(&self, id: &str) -> Result<Option<BlueprintNode>> {
        let q = query("MATCH (b:Blueprint {id: $id}) RETURN b").param("id", id.to_string());

        let mut result = self.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            Ok(Some(Self::node_to_blueprint(&node)?))
        } else {
            Ok(None)
        }
    }

    /// Get a blueprint by slug.
    pub async fn get_blueprint_by_slug(&self, slug: &str) -> Result<Option<BlueprintNode>> {
        let q = query("MATCH (b:Blueprint {slug: $slug}) RETURN b").param("slug", slug.to_string());

        let mut result = self.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            Ok(Some(Self::node_to_blueprint(&node)?))
        } else {
            Ok(None)
        }
    }

    /// Update an existing blueprint (partial update).
    pub async fn update_blueprint(
        &self,
        id: &str,
        req: &UpdateBlueprintRequest,
    ) -> Result<BlueprintNode> {
        // Build dynamic SET clauses
        let mut sets = vec!["b.updated_at = $updated_at"];
        if req.name.is_some() {
            sets.push("b.name = $name");
        }
        if req.description.is_some() {
            sets.push("b.description = $description");
        }
        if req.scope.is_some() {
            sets.push("b.scope = $scope");
        }
        if req.category.is_some() {
            sets.push("b.category = $category");
        }
        if req.difficulty.is_some() {
            sets.push("b.difficulty = $difficulty");
        }
        if req.estimated_time.is_some() {
            sets.push("b.estimated_time = $estimated_time");
        }
        if req.stack.is_some() {
            sets.push("b.stack = $stack");
        }
        if req.tags.is_some() {
            sets.push("b.tags = $tags");
        }
        if req.status.is_some() {
            sets.push("b.status = $status");
        }
        if req.tier1.is_some() {
            sets.push("b.tier1 = $tier1");
        }
        if req.tier2.is_some() {
            sets.push("b.tier2 = $tier2");
        }
        if req.tier3_content.is_some() {
            sets.push("b.tier3_content = $tier3_content");
        }
        if req.source_file.is_some() {
            sets.push("b.source_file = $source_file");
        }
        if req.content_hash.is_some() {
            sets.push("b.content_hash = $content_hash");
        }

        let cypher = format!(
            "MATCH (b:Blueprint {{id: $id}}) SET {} RETURN b",
            sets.join(", ")
        );

        let mut q = query(&cypher)
            .param("id", id.to_string())
            .param("updated_at", chrono::Utc::now().to_rfc3339());

        if let Some(ref name) = req.name {
            q = q.param("name", name.clone());
        }
        if let Some(ref desc) = req.description {
            q = q.param("description", desc.clone());
        }
        if let Some(ref scope) = req.scope {
            q = q.param("scope", scope.to_string());
        }
        if let Some(ref cat) = req.category {
            q = q.param("category", cat.to_string());
        }
        if let Some(ref diff) = req.difficulty {
            q = q.param("difficulty", diff.to_string());
        }
        if let Some(ref et) = req.estimated_time {
            q = q.param("estimated_time", et.clone());
        }
        if let Some(ref stack) = req.stack {
            q = q.param("stack", stack.clone());
        }
        if let Some(ref tags) = req.tags {
            q = q.param("tags", tags.clone());
        }
        if let Some(ref status) = req.status {
            q = q.param("status", status.to_string());
        }
        if let Some(ref t1) = req.tier1 {
            q = q.param("tier1", t1.clone());
        }
        if let Some(ref t2) = req.tier2 {
            q = q.param("tier2", t2.clone());
        }
        if let Some(ref t3) = req.tier3_content {
            q = q.param("tier3_content", t3.clone());
        }
        if let Some(ref sf) = req.source_file {
            q = q.param("source_file", sf.clone());
        }
        if let Some(ref ch) = req.content_hash {
            q = q.param("content_hash", ch.clone());
        }

        let mut result = self
            .graph
            .execute(q)
            .await
            .context("Failed to update Blueprint")?;

        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            Self::node_to_blueprint(&node)
        } else {
            anyhow::bail!("Blueprint not found: {}", id)
        }
    }

    /// Delete a blueprint and all its relations.
    pub async fn delete_blueprint(&self, id: &str) -> Result<()> {
        let q = query(
            r#"
            MATCH (b:Blueprint {id: $id})
            DETACH DELETE b
            "#,
        )
        .param("id", id.to_string());

        let _ = self
            .graph
            .execute(q)
            .await
            .context("Failed to delete Blueprint")?;

        Ok(())
    }

    /// List blueprints with optional filters and pagination.
    /// Returns BlueprintResponse at the requested tier level.
    pub async fn list_blueprints(
        &self,
        query_params: &ListBlueprintsQuery,
    ) -> Result<Vec<BlueprintResponse>> {
        let mut conditions = Vec::new();
        if query_params.scope.is_some() {
            conditions.push("b.scope = $scope");
        }
        if query_params.category.is_some() {
            conditions.push("b.category = $category");
        }
        if query_params.stack.is_some() {
            conditions.push("$stack_filter IN b.stack");
        }
        if query_params.status.is_some() {
            conditions.push("b.status = $status");
        }
        if query_params.search.is_some() {
            conditions.push(
                "(toLower(b.name) CONTAINS toLower($search) OR toLower(b.description) CONTAINS toLower($search))",
            );
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let data_cypher = format!(
            r#"MATCH (b:Blueprint)
            {}
            RETURN b
            ORDER BY b.scope ASC, b.category ASC, b.name ASC
            SKIP $offset
            LIMIT $limit"#,
            where_clause
        );

        let mut data_q = query(&data_cypher);
        if let Some(ref s) = query_params.scope {
            data_q = data_q.param("scope", s.to_string());
        }
        if let Some(ref c) = query_params.category {
            data_q = data_q.param("category", c.to_string());
        }
        if let Some(ref sf) = query_params.stack {
            data_q = data_q.param("stack_filter", sf.to_string());
        }
        if let Some(ref st) = query_params.status {
            data_q = data_q.param("status", st.to_string());
        }
        if let Some(ref se) = query_params.search {
            data_q = data_q.param("search", se.to_string());
        }
        data_q = data_q.param("offset", query_params.offset as i64);
        data_q = data_q.param("limit", query_params.limit as i64);

        let mut result = self.graph.execute(data_q).await?;
        let mut blueprints = Vec::new();
        let tier = query_params.tier;
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            let bp = Self::node_to_blueprint(&node)?;
            blueprints.push(bp.to_response(tier));
        }

        Ok(blueprints)
    }

    // ========================================================================
    // Relations: DEPENDS_ON, PAIRS_WITH (by slug)
    // ========================================================================

    /// Add a relation between two blueprints (by slug).
    pub async fn add_blueprint_relation(
        &self,
        from_slug: &str,
        to_slug: &str,
        relation_type: BlueprintRelationType,
    ) -> Result<()> {
        let rel_type = relation_type.to_string();
        let cypher = format!(
            r#"
            MATCH (a:Blueprint {{slug: $from_slug}})
            MATCH (b:Blueprint {{slug: $to_slug}})
            MERGE (a)-[r:{rel_type}]->(b)
            ON CREATE SET r.created_at = $created_at
            RETURN type(r) AS rel_type
            "#
        );

        let q = query(&cypher)
            .param("from_slug", from_slug.to_string())
            .param("to_slug", to_slug.to_string())
            .param("created_at", chrono::Utc::now().to_rfc3339());

        let mut result = self
            .graph
            .execute(q)
            .await
            .context("Failed to add blueprint relation")?;

        if result.next().await?.is_none() {
            anyhow::bail!(
                "Cannot create relation: blueprint '{}' or '{}' not found",
                from_slug,
                to_slug
            );
        }
        Ok(())
    }

    /// Remove a relation between two blueprints (by slug).
    pub async fn remove_blueprint_relation(
        &self,
        from_slug: &str,
        to_slug: &str,
        relation_type: BlueprintRelationType,
    ) -> Result<()> {
        let rel_type = relation_type.to_string();
        let cypher = format!(
            r#"
            MATCH (a:Blueprint {{slug: $from_slug}})-[r:{rel_type}]->(b:Blueprint {{slug: $to_slug}})
            DELETE r
            "#
        );

        let q = query(&cypher)
            .param("from_slug", from_slug.to_string())
            .param("to_slug", to_slug.to_string());

        let _ = self.graph.execute(q).await?;
        Ok(())
    }

    /// Get transitive dependencies (forward DEPENDS_ON*1..3), returns BlueprintResponse.
    pub async fn get_blueprint_dependencies(&self, slug: &str) -> Result<Vec<BlueprintResponse>> {
        let q = query(
            r#"
            MATCH (b:Blueprint {slug: $slug})-[:DEPENDS_ON*1..3]->(dep:Blueprint)
            WHERE dep.status <> 'archived'
            RETURN DISTINCT dep
            "#,
        )
        .param("slug", slug.to_string());

        let mut result = self.graph.execute(q).await?;
        let mut deps = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("dep")?;
            let bp = Self::node_to_blueprint(&node)?;
            deps.push(bp.to_response(1));
        }
        Ok(deps)
    }

    /// Get transitive dependents (reverse DEPENDS_ON*1..3), returns BlueprintResponse.
    pub async fn get_blueprint_dependents(&self, slug: &str) -> Result<Vec<BlueprintResponse>> {
        let q = query(
            r#"
            MATCH (b:Blueprint {slug: $slug})<-[:DEPENDS_ON*1..3]-(dep:Blueprint)
            WHERE dep.status <> 'archived'
            RETURN DISTINCT dep
            "#,
        )
        .param("slug", slug.to_string());

        let mut result = self.graph.execute(q).await?;
        let mut deps = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("dep")?;
            let bp = Self::node_to_blueprint(&node)?;
            deps.push(bp.to_response(1));
        }
        Ok(deps)
    }

    /// Get paired blueprints (bidirectional PAIRS_WITH), returns BlueprintResponse.
    pub async fn get_blueprint_pairs(&self, slug: &str) -> Result<Vec<BlueprintResponse>> {
        let q = query(
            r#"
            MATCH (b:Blueprint {slug: $slug})-[:PAIRS_WITH]-(pair:Blueprint)
            WHERE pair.status <> 'archived'
            RETURN DISTINCT pair
            "#,
        )
        .param("slug", slug.to_string());

        let mut result = self.graph.execute(q).await?;
        let mut pairs = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("pair")?;
            let bp = Self::node_to_blueprint(&node)?;
            pairs.push(bp.to_response(1));
        }
        Ok(pairs)
    }

    // ========================================================================
    // APPLIES_TO relation (Blueprint → Project)
    // ========================================================================

    /// Link a blueprint to a project (APPLIES_TO with relevance score).
    pub async fn link_blueprint_to_project(
        &self,
        slug: &str,
        project_id: &str,
        relevance: f64,
    ) -> Result<()> {
        let q = query(
            r#"
            MATCH (b:Blueprint {slug: $slug})
            MATCH (p:Project {id: $project_id})
            MERGE (b)-[r:APPLIES_TO]->(p)
            ON CREATE SET r.relevance = $relevance, r.resolved_at = $now
            ON MATCH SET r.relevance = $relevance, r.resolved_at = $now
            RETURN type(r) AS rel
            "#,
        )
        .param("slug", slug.to_string())
        .param("project_id", project_id.to_string())
        .param("relevance", relevance)
        .param("now", chrono::Utc::now().to_rfc3339());

        let _ = self
            .graph
            .execute(q)
            .await
            .context("Failed to link blueprint to project")?;
        Ok(())
    }

    /// Unlink a blueprint from a project.
    pub async fn unlink_blueprint_from_project(&self, slug: &str, project_id: &str) -> Result<()> {
        let q = query(
            r#"
            MATCH (b:Blueprint {slug: $slug})-[r:APPLIES_TO]->(p:Project {id: $project_id})
            DELETE r
            "#,
        )
        .param("slug", slug.to_string())
        .param("project_id", project_id.to_string());

        let _ = self.graph.execute(q).await?;
        Ok(())
    }

    /// Get all blueprints linked to a project, returns BlueprintResponse.
    pub async fn get_project_blueprints(&self, project_id: &str) -> Result<Vec<BlueprintResponse>> {
        let q = query(
            r#"
            MATCH (b:Blueprint)-[r:APPLIES_TO]->(p:Project {id: $project_id})
            WHERE b.status <> 'archived'
            RETURN b, r.relevance AS relevance
            ORDER BY r.relevance DESC
            "#,
        )
        .param("project_id", project_id.to_string());

        let mut result = self.graph.execute(q).await?;
        let mut blueprints = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("b")?;
            let bp = Self::node_to_blueprint(&node)?;
            blueprints.push(bp.to_response(2)); // tier 2 for project context
        }
        Ok(blueprints)
    }
}

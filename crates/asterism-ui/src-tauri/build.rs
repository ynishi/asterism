//! Build script — invokes the Tauri build and regenerates the shared
//! TypeScript bindings.
//!
//! Every Command / Query / Response DTO **the UI consumes** is rendered
//! to `../src/bindings.ts` via `schema-bridge`. Hand-written TypeScript
//! types are avoided so the two sides never drift apart.
//!
//! The list below is explicit, not exhaustive, and nothing checks it
//! against `asterism-contract`. Three deliberate omissions today. The
//! first two are one shape — a surface the UI will not reach — and the
//! third is a surface it has not reached yet:
//!
//! - the diagnostics *read* types (`ListDiagQuery` / `DiagDto`) derive
//!   `SchemaBridge` but stay out, because reading `/asterism/diag` is
//!   HTTP-only and the UI never consumes it. The *write* command
//!   (`RecordDiagCommand`) is in: the webview is itself a diagnostic
//!   source (`lib/diag.ts`);
//! - the series Strategy types (`SeriesStrategyDto` and its three
//!   commands) stay out although their IPC commands now exist (#136):
//!   no screen consumes them, and the eventual UI is about promoting
//!   one series to a real Group, which is a different shape from
//!   editing a rule. Until a screen imports one, exporting the shape
//!   would only give it something to drift from;
//! - the forge's types (`asterism_contract::forge`) stayed out until a
//!   screen imported one, and that was a different reason from the two
//!   above rather than a weaker one. A screen is the whole reason that
//!   surface is being built — HTTP first, the UI on it — and
//!   exporting a shape before the screen that shapes it is how a
//!   binding drifts from what the screen actually needs. A type enters
//!   this list in the change that consumes it.
//!
//! #153 is that change for five of them. The shared-lines panel
//! (#148 decision 16) lists `ForgeLineDto`, shows what is on a line
//! with `ForgeEntryStateDto`, and counts a line's change points out of
//! `ForgeLineHistoryDto` — which is how the panel shows the difference
//! between a line published as it stands and one whose chain was
//! re-enacted, and which drags in the `ForgeChangePointDto` /
//! `ForgeChangeRowDto` it is made of. The rest of the forge surface is
//! still not exported, on the rule above and not by oversight: the
//! panel does not open work, push rounds or hold conversations, and
//! the screen that does is the change that adds those.

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AddAssetToGroupCommand,
    AppendMessageCommand, ArchivePersonaCommand, ArchiveThreadCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, CancelJobCommand, CreateDirCommand,
    CreateDispatchCommand, CreateGroupCommand, CreateMaterialLayerCommand, CreateModalityCommand,
    CreateQueryGroupCommand, CreateSavedQueryCommand, CreateThreadCommand, DeclareAssetMetaCommand,
    DeclareProvenanceCommand, DeclareSourceTypeCommand, DeleteAssetCommentCommand,
    DeleteChapterMarkCommand, DeleteDirCommand, DeleteMaterialLayerCommand,
    DeleteMaterialMarkCommand, DeleteMessageCommand, DeleteModalityCommand,
    DeletePersonaProfileCommand, DeletePersonaThemeCommand, DeleteSavedQueryCommand,
    DeleteSessionCommand, DeleteThreadCommand, DetachTagBatchCommand, DetachTagBatchResult,
    DetachTagCommand, DispatchRunCommand, EditAssetCommentCommand, EditChapterMarkCommand,
    EditMaterialMarkCommand, EmptyTrashResult, LinkGroupCommand, MergeAssetsCommand,
    MoveDirCommand, MoveGroupToDirCommand, OrganizeByLocationCommand, OrganizeByLocationResult,
    PasteImageImportCommand, PatchSessionMetadataCommand, PostAssetCommentCommand,
    PostChapterMarkCommand, PostMaterialMarkCommand, PromoteSnapshotToGroupCommand,
    PromoteSnapshotToGroupResult, PromoteTagToGroupCommand, PromoteTagToGroupResult,
    PromoteVolatileSelectionCommand, PurgeAssetCommand, PurgeGroupCommand, PurgePersonaCommand,
    RebuildEdgesCommand, RecordDiagCommand, RecordEventCommand, RedispatchCommand,
    RegisterPersonaCommand, RemoveAssetFromGroupCommand, RenameDirCommand, RenameGroupCommand,
    RenameSavedQueryCommand, RenameSessionCommand, ReorderGroupAssetsCommand,
    ReorderGroupChildrenCommand, ReorderPersonasCommand, ResetSettingCommand,
    ResolveDuplicateConflictCommand, RestoreAssetCommand, RestoreGroupCommand,
    RestorePersonaCommand, SetDefaultMaterialLayerCommand, SetPersonaProfileCommand,
    SetPersonaThemeCommand, SetSettingCommand, TrashAssetCommand, TrashGroupCommand,
    TrashPersonaCommand, UnlinkGroupCommand, UpdateAssetMetaBatchCommand,
    UpdateAssetMetaBatchResult, UpdateAssetMetaCommand, UpdateModalityCommand,
    UpdateQueryGroupQueryCommand,
};
use asterism_contract::dto::{
    AssertedSourceTypeDto, AssetCardDto, AssetCommentDto, AssetCountEntryDto, AssetDetailDto,
    AssetDto, AssetIndexEntryDto, AssetIndexPageDto, AssetPageDto, AssetSourceTypeDto,
    AssetTextDto, ChapterMarkDto, ConstellationItemDto, DirDto, DispatchDto, DuplicateAxis,
    DuplicateConflictDto, DuplicateGroupDto, DuplicateReportDto, DuplicateResolutionDto, EdgeDto,
    EventDto, GroupDto, GroupLinkDto, GroupSummaryDto, JobDto, JobKindSnapshotDto, JobsSnapshotDto,
    LineageEdgeDto, LineageNodeDto, LineageViewDto, MaterialLayerDto, MaterialLayerViewDto,
    MaterialMarkDto, MergeAssetsDto, MergeRefusalDto, MergeTotalsDto, MergeWarningDto, MessageDto,
    MessageRefDto, ModalityDefDto, PersonaDto, PersonaProfileDto, PersonaThemeDto,
    ProvenanceViewDto, RetrievedIdsDto, RetrievedPageDto, SampledPageDto, SavedQueryDto,
    SessionDto, SessionPageDto, SettingDto, SettingLayerDto, SnapshotDto, TagCountDto, TagDto,
    TagSuggestionDto, ThreadAnchorDto, ThreadDto, VideoPreviewDto, VisualModelStatusDto,
};
use asterism_contract::forge::{
    ForgeChangePointDto, ForgeChangeRowDto, ForgeEntryStateDto, ForgeLineDto, ForgeLineHistoryDto,
};
use asterism_contract::query::{
    GetAssetDetailQuery, GetJobStatusQuery, ListAssetsQuery, ListEventsQuery, RandomAssetsQuery,
    SearchAssetsQuery,
};
use schema_bridge::{SchemaBridge as _, export_types};

fn main() {
    export_types!(
        "../src/bindings.ts",
        // Command
        RegisterPersonaCommand,
        ArchivePersonaCommand,
        TrashPersonaCommand,
        RestorePersonaCommand,
        PurgePersonaCommand,
        AddAssetCommand,
        AddAssetBatchCommand,
        AddAssetBatchResult,
        UpdateAssetMetaCommand,
        UpdateAssetMetaBatchCommand,
        UpdateAssetMetaBatchResult,
        TrashAssetCommand,
        RestoreAssetCommand,
        PurgeAssetCommand,
        // `EmptyTrashCommand` carries no fields, so it has no
        // `SchemaBridge` derive to export; the UI sends `{}` and only
        // needs the result type.
        EmptyTrashResult,
        DeclareProvenanceCommand,
        DeclareAssetMetaCommand,
        // The source-type row on the detail pane (#108): the assert /
        // retract verb and the read its three states come from.
        DeclareSourceTypeCommand,
        AssetSourceTypeDto,
        AssertedSourceTypeDto,
        RebuildEdgesCommand,
        // The settings screen's Maintenance section consumes these two
        // (#136); the other maintenance verbs' inputs travel as plain
        // strings and string lists and need no type here.
        OrganizeByLocationCommand,
        OrganizeByLocationResult,
        CancelJobCommand,
        CreateGroupCommand,
        TrashGroupCommand,
        RestoreGroupCommand,
        PurgeGroupCommand,
        AddAssetToGroupCommand,
        RemoveAssetFromGroupCommand,
        ReorderGroupAssetsCommand,
        RenameGroupCommand,
        MoveGroupToDirCommand,
        LinkGroupCommand,
        UnlinkGroupCommand,
        ReorderGroupChildrenCommand,
        CreateDirCommand,
        RenameDirCommand,
        MoveDirCommand,
        DeleteDirCommand,
        SetPersonaThemeCommand,
        DeletePersonaThemeCommand,
        SetPersonaProfileCommand,
        DeletePersonaProfileCommand,
        CreateModalityCommand,
        UpdateModalityCommand,
        DeleteModalityCommand,
        SetSettingCommand,
        ResetSettingCommand,
        RenameSessionCommand,
        PatchSessionMetadataCommand,
        DeleteSessionCommand,
        PasteImageImportCommand,
        ReorderPersonasCommand,
        AttachTagCommand,
        DetachTagCommand,
        AttachTagBatchCommand,
        AttachTagBatchResult,
        DetachTagBatchCommand,
        DetachTagBatchResult,
        PromoteTagToGroupCommand,
        PromoteTagToGroupResult,
        CreateDispatchCommand,
        PromoteSnapshotToGroupCommand,
        PromoteSnapshotToGroupResult,
        PromoteVolatileSelectionCommand,
        CreateQueryGroupCommand,
        UpdateQueryGroupQueryCommand,
        DispatchRunCommand,
        RedispatchCommand,
        CreateSavedQueryCommand,
        RenameSavedQueryCommand,
        DeleteSavedQueryCommand,
        PostAssetCommentCommand,
        EditAssetCommentCommand,
        DeleteAssetCommentCommand,
        PostMaterialMarkCommand,
        EditMaterialMarkCommand,
        DeleteMaterialMarkCommand,
        CreateMaterialLayerCommand,
        SetDefaultMaterialLayerCommand,
        DeleteMaterialLayerCommand,
        PostChapterMarkCommand,
        EditChapterMarkCommand,
        DeleteChapterMarkCommand,
        RecordEventCommand,
        RecordDiagCommand,
        ResolveDuplicateConflictCommand,
        MergeAssetsCommand,
        // Query
        ListAssetsQuery,
        SearchAssetsQuery,
        RandomAssetsQuery,
        GetAssetDetailQuery,
        GetJobStatusQuery,
        ListEventsQuery,
        // Response
        PersonaDto,
        PersonaThemeDto,
        PersonaProfileDto,
        AssetCardDto,
        AssetPageDto,
        RetrievedPageDto,
        RetrievedIdsDto,
        SampledPageDto,
        AssetIndexEntryDto,
        AssetIndexPageDto,
        AssetDto,
        TagDto,
        TagSuggestionDto,
        VisualModelStatusDto,
        TagCountDto,
        SessionDto,
        SessionPageDto,
        GroupDto,
        GroupSummaryDto,
        DirDto,
        GroupLinkDto,
        AssetTextDto,
        EdgeDto,
        ConstellationItemDto,
        ProvenanceViewDto,
        LineageNodeDto,
        LineageEdgeDto,
        LineageViewDto,
        AssetDetailDto,
        JobDto,
        JobKindSnapshotDto,
        JobsSnapshotDto,
        SnapshotDto,
        DispatchDto,
        SavedQueryDto,
        AssetCountEntryDto,
        DuplicateAxis,
        DuplicateGroupDto,
        DuplicateReportDto,
        DuplicateConflictDto,
        DuplicateResolutionDto,
        MergeAssetsDto,
        MergeRefusalDto,
        MergeWarningDto,
        MergeTotalsDto,
        ModalityDefDto,
        SettingLayerDto,
        SettingDto,
        AssetCommentDto,
        MaterialMarkDto,
        MaterialLayerDto,
        ChapterMarkDto,
        MaterialLayerViewDto,
        EventDto,
        VideoPreviewDto,
        // App-level Threads.
        CreateThreadCommand,
        ArchiveThreadCommand,
        DeleteThreadCommand,
        AppendMessageCommand,
        DeleteMessageCommand,
        ThreadDto,
        ThreadAnchorDto,
        MessageDto,
        MessageRefDto,
        // The forge, as far as the shared-lines panel reads it (#153).
        ForgeLineDto,
        ForgeEntryStateDto,
        ForgeLineHistoryDto,
        ForgeChangePointDto,
        ForgeChangeRowDto,
    )
    .expect("failed to export TS bindings from asterism-contract");
    tauri_build::build()
}

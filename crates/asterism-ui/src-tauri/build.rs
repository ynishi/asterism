//! Build script — invokes the Tauri build and regenerates the shared
//! TypeScript bindings.
//!
//! Every Command / Query / Response DTO in `asterism-contract` is
//! rendered to `../src/bindings.ts` via `schema-bridge`, but for the
//! few `tests/export_parity.rs` records a reason against. Hand-written
//! TypeScript types are avoided so the two sides never drift apart.
//!
//! **One crate, and that is the point.** The frontend has one
//! vocabulary because there is one boundary between it and this
//! binary. A shape arriving from a boundary further out — the teams
//! wire, which a member's client and a team server speak — is mapped
//! to a contract type by the command that fetches it, rather than
//! exported from here as a second source. Exporting one would hand
//! every screen a second vocabulary and leave `export_parity` guarding
//! half a list, which is what `asterism-contract::teams` exists to
//! avoid.
//!
//! **The list is a projection of the contract, not a subset a screen
//! asked for.** It used to be the second: a type entered the list in
//! the change that consumed it, on the ground that "exporting a shape
//! before the screen that shapes it is how a binding drifts from what
//! the screen actually needs". #175 withdrew that. There is nothing
//! here for a projection to drift *from* — `schema-bridge` generates
//! the file and no line of it is written by hand — and the rule's real
//! effect was to make the binding layer wait on the UI: a verb
//! reachable over HTTP and over IPC could not be named in TypeScript
//! until somebody edited this file. #173 settled the same question for
//! the other two transports, which owe each other every verb whatever
//! any screen consumes.
//!
//! What the rule left behind is what the withdrawal cost. This doc said
//! "three deliberate omissions" and named three; the contract went on
//! growing types this list did not carry, and most of them were named
//! nowhere at all — because, as the same paragraph admitted, nothing
//! ever compared the two.
//!
//! `tests/export_parity.rs` does now. Its `NOT_PROJECTED` holds what
//! stays out and why, and it writes both sides to `exported-types.txt`
//! beside it — a tracked file, so a shape changing sides arrives as a
//! diff to read rather than as a number somebody has to re-derive. No
//! count lives here, for the reason the old one is a lesson in.
//!
//! An absence needs a reason about **reach** rather than about timing.
//! "No screen consumes it yet" is a sentence about today that nothing
//! maintains; "the app has no path to call it" is a fact about the
//! tree, and it is the shape every surviving entry takes.

use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AddAssetToGroupCommand,
    AppendMessageCommand, ArchivePersonaCommand, ArchiveThreadCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, BatchGroupMembershipCommand, CancelJobCommand,
    ConflictResolution, CreateDirCommand, CreateDispatchCommand, CreateGroupCommand,
    CreateMaterialLayerCommand, CreateModalityCommand, CreateQueryGroupCommand,
    CreateSavedQueryCommand, CreateSeriesStrategyCommand, CreateSnapshotCommand,
    CreateThreadCommand, DeclareAssetMetaCommand, DeclareProvenanceCommand,
    DeclareSourceTypeCommand, DeleteAssetCommentCommand, DeleteChapterMarkCommand,
    DeleteDirCommand, DeleteMaterialLayerCommand, DeleteMaterialMarkCommand, DeleteMessageCommand,
    DeleteModalityCommand, DeletePersonaProfileCommand, DeletePersonaThemeCommand,
    DeleteSavedQueryCommand, DeleteSeriesStrategyCommand, DeleteSessionCommand, DeleteTagCommand,
    DeleteTagResult, DeleteThreadCommand, DetachTagBatchCommand, DetachTagBatchResult,
    DetachTagCommand, DispatchRunCommand, EditAssetCommentCommand, EditChapterMarkCommand,
    EditMaterialMarkCommand, EmptyTrashResult, GroupMembershipEntry, LinkGroupCommand,
    MergeAssetsCommand, MergeGroupsCommand, MergeTagsCommand, MergeTagsResult, MoveDirCommand,
    MoveGroupToDirCommand, OnDuplicate, OrganizeByLocationCommand, OrganizeByLocationResult,
    PasteImageImportCommand, PatchSessionMetadataCommand, PostAssetCommentCommand,
    PostChapterMarkCommand, PostMaterialMarkCommand, PromoteSnapshotToGroupCommand,
    PromoteSnapshotToGroupResult, PromoteTagToGroupCommand, PromoteTagToGroupResult,
    PromoteVolatileSelectionCommand, PurgeAssetCommand, PurgeGroupCommand, PurgePersonaCommand,
    RebuildEdgesCommand, RecordDiagCommand, RecordEventCommand, RedispatchCommand,
    RegisterPersonaCommand, RemoveAssetFromGroupCommand, RenameDirCommand, RenameGroupCommand,
    RenameSavedQueryCommand, RenameSessionCommand, RenameTagCommand, ReorderGroupAssetsCommand,
    ReorderGroupChildrenCommand, ReorderPersonasCommand, ResetSettingCommand,
    ResolveDuplicateConflictCommand, RestoreAssetCommand, RestoreGroupCommand,
    RestorePersonaCommand, SetDefaultMaterialLayerCommand, SetPersonaProfileCommand,
    SetPersonaThemeCommand, SetSettingCommand, TrashAssetCommand, TrashGroupCommand,
    TrashPersonaCommand, UnlinkGroupCommand, UpdateAssetMetaBatchCommand,
    UpdateAssetMetaBatchResult, UpdateAssetMetaCommand, UpdateModalityCommand,
    UpdateQueryGroupQueryCommand, UpdateSeriesStrategyCommand,
};
use asterism_contract::dto::{
    AssertedSourceTypeDto, AssetCardDto, AssetCommentDto, AssetCountEntryDto, AssetDetailDto,
    AssetDto, AssetIndexEntryDto, AssetIndexPageDto, AssetPageDto, AssetSourceTypeDto,
    AssetTextDto, ChapterMarkDto, ConstellationItemDto, DirDto, DispatchDto, DuplicateAxis,
    DuplicateConflictDto, DuplicateGroupDto, DuplicateReportDto, DuplicateResolutionDto, EdgeDto,
    EventDto, GroupDto, GroupLinkDto, GroupSummaryDto, HeadStatusDto, JobDto, JobKindSnapshotDto,
    JobsSnapshotDto, LineageEdgeDto, LineageNodeDto, LineageViewDto, MaterialLayerDto,
    MaterialLayerViewDto, MaterialMarkDto, MergeAssetsDto, MergeRefusalDto, MergeTotalsDto,
    MergeWarningDto, MessageDto, MessageRefDto, ModalityDefDto, ObservationDto, PersonaDto,
    PersonaProfileDto, PersonaThemeDto, ProvenanceViewDto, RetrievedIdsDto, RetrievedPageDto,
    RulingReadinessDto, SampledPageDto, SavedQueryDto, SeriesStrategyDto, SessionDto,
    SessionPageDto, SettingDto, SettingLayerDto, SnapshotDto, TagCountDto, TagDto,
    TagSuggestionDto, ThreadAnchorDto, ThreadDto, TrainedHeadRunDto, VideoPreviewDto,
    VisualModelStatusDto,
};
use asterism_contract::forge::{
    AmendForgeMessageCommand, CloseForgePursuitCommand, ForgeAnchorDto, ForgeChangePointDto,
    ForgeChangeRowDto, ForgeCloseDto, ForgeCollisionDto, ForgeDiscardedDto, ForgeEntryStateDto,
    ForgeLineActCommand, ForgeLineDto, ForgeLineHistoryDto, ForgeMessageDto, ForgeOpDto,
    ForgePursuitActCommand, ForgePursuitDto, ForgeResolvedDto, ForgeRevisionDto, ForgeRoundDto,
    ForgeStrategyDto, ForgeThreadDto, OpenForgeLineCommand, OpenForgePursuitCommand,
    OpenForgeThreadCommand, PushForgeRoundCommand, RenameForgeLineCommand,
    RenameForgeThreadCommand, SayInForgeThreadCommand, SetForgeLineStrategyCommand,
};
use asterism_contract::query::{
    GetAssetDetailQuery, GetJobStatusQuery, ListAssetsQuery, ListEventsQuery,
    ListObservationsQuery, RandomAssetsQuery, SearchAssetsQuery, TagMatch,
};
use asterism_contract::sort::{SortOrder, SortSpec, SortTarget};
use asterism_contract::teams::{
    MyTeamDto, MyTeamsDto, PromotedAssetDto, StoredTeamConnectDto, StoredTeamConnectOutcome,
    StoredTeamConnectionDto, TeamCreatedDto, TeamDeviceTokenDto, TeamDeviceTokensDto,
    TeamLedgerEventDto, TeamLedgerPageDto, TeamRosterDto, TeamRosterMemberDto, TeamRosterViewerDto,
    TeamSubjectRefDto,
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
        // The model panel's other half (#130): what the promotion
        // pointer names, and what a next training run would learn from.
        HeadStatusDto,
        TrainedHeadRunDto,
        RulingReadinessDto,
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
        // The forge, whole.
        ForgeLineDto,
        ForgeEntryStateDto,
        ForgeLineHistoryDto,
        ForgeChangePointDto,
        ForgeChangeRowDto,
        ForgePursuitDto,
        ForgeRoundDto,
        ForgeOpDto,
        ForgeCloseDto,
        ForgeCollisionDto,
        ForgeResolvedDto,
        ForgeDiscardedDto,
        ForgeStrategyDto,
        ForgeThreadDto,
        ForgeMessageDto,
        ForgeRevisionDto,
        ForgeAnchorDto,
        OpenForgeLineCommand,
        RenameForgeLineCommand,
        SetForgeLineStrategyCommand,
        ForgeLineActCommand,
        OpenForgePursuitCommand,
        PushForgeRoundCommand,
        CloseForgePursuitCommand,
        ForgePursuitActCommand,
        OpenForgeThreadCommand,
        SayInForgeThreadCommand,
        AmendForgeMessageCommand,
        RenameForgeThreadCommand,
        // Tag administration. Delete and merge answer with a result
        // shape saying what the write moved; rename answers with the
        // tag, so it has none.
        RenameTagCommand,
        DeleteTagCommand,
        DeleteTagResult,
        MergeTagsCommand,
        MergeTagsResult,
        // Series Strategy — the registered derivation rules (#136).
        SeriesStrategyDto,
        CreateSeriesStrategyCommand,
        UpdateSeriesStrategyCommand,
        DeleteSeriesStrategyCommand,
        // Snapshots, group membership in bulk, and the two vocabularies a
        // caller states rather than receives.
        CreateSnapshotCommand,
        BatchGroupMembershipCommand,
        GroupMembershipEntry,
        MergeGroupsCommand,
        ConflictResolution,
        OnDuplicate,
        // The observation stream, which `list_observations` serves over
        // IPC as well as HTTP.
        ObservationDto,
        ListObservationsQuery,
        // The sort vocabulary, and the tag-match mode beside it. Both
        // already reach TypeScript inlined inside `ListAssetsQuery`
        // (`sort: { target: … }`, `tag_match: 'any' | 'all'`); named
        // here so a caller can hold one in a variable rather than
        // spelling the union again at each site.
        //
        // This does not make the UI's own `SortTarget` a copy to be
        // reconciled. That union differs from the contract's in both
        // directions on purpose — it carries `relevance`, which has no
        // wire token, and omits `updated_at` / `rating`, which the grid
        // never asks for — and `lib/stores/filter.svelte.ts` states both
        // at the union itself.
        SortTarget,
        SortOrder,
        SortSpec,
        TagMatch,
        // The team plane, as this app's own boundary carries it. What
        // the wire carries is what a member's client and a team server
        // say to each other; these are what a command hands a screen,
        // and the mapping between them is the command's.
        TeamLedgerPageDto,
        TeamLedgerEventDto,
        TeamSubjectRefDto,
        TeamRosterDto,
        TeamRosterMemberDto,
        TeamRosterViewerDto,
        TeamCreatedDto,
        PromotedAssetDto,
        MyTeamsDto,
        MyTeamDto,
        TeamDeviceTokensDto,
        TeamDeviceTokenDto,
        StoredTeamConnectionDto,
        StoredTeamConnectDto,
        StoredTeamConnectOutcome,
    )
    .expect("failed to export TS bindings from asterism-contract");
    tauri_build::build()
}

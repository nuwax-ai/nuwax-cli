-- 平台使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS agent_platform CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
-- 数据表组件使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS agent_custom_table CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

GRANT ALL PRIVILEGES ON agent_platform.* TO 'agent_platform'@'%';
GRANT ALL PRIVILEGES ON agent_custom_table.* TO 'agent_platform'@'%';
FLUSH PRIVILEGES;


USE agent_platform;

create table agent_component_config
(
    id            bigint auto_increment
        primary key,
    _tenant_id    bigint   default 1                 not null comment '商户ID',
    name          varchar(64)                        null comment '节点名称',
    icon          varchar(255)                       null comment '组件图标',
    description   text                               null comment '组件描述',
    agent_id      bigint                             null comment 'AgentID',
    type          varchar(64)                        not null comment '组件类型',
    target_id     bigint                             null comment '关联的组件ID',
    bind_config   json                               null comment '组件绑定配置',
    exception_out tinyint  default 0                 not null comment '异常是否抛出，中断主要流程',
    fallback_msg  text                               null comment '异常时兜底内容',
    modified      datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created       datetime default CURRENT_TIMESTAMP not null
)
    comment '智能体组件配置';

create table agent_config
(
    id                    bigint auto_increment comment '智能体ID'
        primary key,
    uid                   varchar(64)                                      not null comment 'agent唯一标识',
    type                  varchar(32)            default 'ChatBot'         not null comment '智能体类型',
    _tenant_id            bigint                 default -1                not null comment '商户ID',
    space_id              bigint                                           null comment '空间ID',
    creator_id            bigint                                           not null comment '创建者ID',
    name                  varchar(64)                                      not null comment 'Agent名称',
    description           varchar(2000)                                    null comment 'Agent描述',
    icon                  varchar(255)                                     null comment '图标地址',
    system_prompt         mediumtext                                       null comment '系统提示词',
    user_prompt           text                                             null comment '用户消息提示词，{{AGENT_USER_MSG}}引用用户消息',
    open_suggest          enum ('Open', 'Close') default 'Open'            not null comment '是否开启问题建议',
    suggest_prompt        text                                             null comment '用户问题建议',
    opening_chat_msg      mediumtext                                       null comment '首次打开聊天框自动回复消息',
    opening_guid_question json                                             null comment '开场引导问题',
    open_long_memory      enum ('Open', 'Close') default 'Open'            not null comment '是否开启长期记忆',
    open_scheduled_task   varchar(32)                                      null comment '开启定时任务',
    publish_status        varchar(32)            default 'Developing'      not null comment 'Agent发布状态',
    dev_conversation_id   bigint                                           null,
    expand_page_area      tinyint                default 0                 not null comment '默认展开页面区域',
    hide_chat_area        tinyint                default 0                 not null comment '隐藏对话框',
    yn                    tinyint                default 0                 not null comment '逻辑删除，1为删除',
    modified              datetime               default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created               datetime               default CURRENT_TIMESTAMP not null comment '创建时间'
);

create index idx_space_id
    on agent_config (space_id);

create table agent_temp_chat
(
    id            bigint auto_increment
        primary key,
    _tenant_id    bigint                             not null,
    user_id       bigint                             not null comment '创建链接的用户ID',
    agent_id      bigint                             not null,
    chat_key      varchar(64)                        not null comment '临时会话标识',
    require_login tinyint  default 1                 not null comment '是否需要登录 1 是，0 否',
    expire        datetime                           null,
    modified      datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created       datetime default CURRENT_TIMESTAMP not null
);

create table card
(
    id        bigint auto_increment
        primary key,
    card_key  varchar(32)                        not null comment '卡片唯一标识，与前端组件做关联',
    name      varchar(64)                        not null comment '卡片名称',
    image_url varchar(255)                       null comment '卡片示例图片地址',
    args      json                               null comment '卡片参数',
    modified  datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created   datetime default CURRENT_TIMESTAMP not null
);

create table config_history
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint                               not null,
    op_user_id  bigint                               null comment '操作用户',
    target_id   bigint                               not null comment '目标对象ID',
    target_type enum ('Agent', 'Plugin', 'Workflow') not null comment '目标对象类型',
    type        varchar(64)                          not null comment '历史记录类型',
    config      json                                 null comment '当时的配置',
    description varchar(255)                         null comment '变更描述',
    modified    datetime default CURRENT_TIMESTAMP   not null on update CURRENT_TIMESTAMP comment '更新时间',
    created     datetime default CURRENT_TIMESTAMP   not null
);

create table content_i18n
(
    id        bigint auto_increment comment 'ID'
        primary key,
    model     varchar(32)                        not null comment '业务模块标记',
    mid       varchar(32)                        not null comment '业务模块ID',
    lang      varchar(16)                        not null comment '语言，中文：zh-cn，英文:en-us',
    field_key varchar(64)                        not null comment '业务表字段',
    content   mediumtext                         null comment '具体内容',
    modified  datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created   datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    constraint uk_lang_content
        unique (model, mid, lang, field_key)
)
    comment '内容国际化表';

create table conversation
(
    id            bigint auto_increment
        primary key,
    _tenant_id    bigint                                not null comment '商户ID',
    uid           varchar(64)                           not null comment '会话唯一标识',
    user_id       bigint                                not null comment '用户ID',
    agent_id      bigint                                not null comment '智能体ID',
    topic         varchar(255)                          not null comment '主题',
    summary       mediumtext                            null comment '汇总',
    variables     json                                  null comment '用户输入的变量值',
    dev_mode      tinyint     default 0                 not null,
    topic_updated tinyint     default 0                 not null,
    type          varchar(32) default 'Chat'            not null comment '会话类型，Chat对话；Task 定时任务',
    task_id       varchar(64)                           null comment '对应的任务ID',
    task_status   varchar(32)                           null comment '任务状态',
    task_cron     varchar(32)                           null comment '任务配置',
    modified      datetime    default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created       datetime    default CURRENT_TIMESTAMP not null comment '创建时间'
)
    comment '会话表';

create index idx_uid
    on conversation (uid);

create index idx_user_id
    on conversation (user_id);

create table custom_field_definition
(
    id                bigint auto_increment comment '主键ID'
        primary key,
    _tenant_id        bigint                               not null comment '租户ID',
    space_id          bigint                               not null comment '所属空间ID',
    table_id          bigint                               not null comment '关联的表ID',
    field_name        varchar(64)                          not null comment '字段名',
    field_description varchar(200)                         null comment '字段描述',
    field_type        tinyint    default 1                 not null comment '字段类型：1:String;2:Integer;3:Number;4:Boolean;5:Date',
    nullable_flag     tinyint(1) default 1                 not null comment '是否可为空：1-可空 -1-非空',
    default_value     varchar(255)                         null comment '默认值',
    unique_flag       tinyint(1) default -1                not null comment '是否唯一：1-唯一 -1-非唯一',
    enabled_flag      tinyint(1) default 1                 not null comment '是否启用：1-启用 -1-禁用',
    sort_index        int                                  not null comment '字段顺序',
    created           datetime   default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id        bigint                               null comment '创建人id',
    creator_name      varchar(64)                          null comment '创建人',
    modified          datetime   default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id       bigint                               null comment '最后修改人id',
    modified_name     varchar(64)                          null comment '最后修改人',
    yn                tinyint    default 1                 null comment '逻辑标记,1:有效;-1:无效',
    system_field_flag tinyint    default -1                not null comment '是否系统字段;1:系统字段;-1:否',
    field_str_len     int                                  null comment '字符串字段长度,可空,比如字符串,可以指定长度使用',
    constraint uk_table_field
        unique (table_id, field_name)
)
    comment '自定义字段定义';

create index idx_table_id
    on custom_field_definition (table_id);

create table custom_page_build
(
    id                   bigint auto_increment comment '主键ID'
        primary key,
    project_id           bigint                             not null comment '项目ID',
    dev_running          tinyint  default -1                not null comment '开发服务器运行标记,1:运行中;-1:未运行',
    dev_pid              int                                null comment '开发服务器进程ID',
    dev_port             int                                null comment '开发服务器端口号',
    dev_time             datetime                           null comment '开发服务器启动时间',
    last_keep_alive_time datetime                           null comment '最后保活时间',
    build_running        tinyint  default -1                not null comment '线上运行标记,1:运行中;-1:未运行',
    build_time           datetime                           null comment '构建发布时间',
    build_version        int                                null comment '发布的版本号',
    code_version         int                                not null comment '代码版本',
    version_info         json                               null comment '版本信息',
    last_chat_model_id   bigint                             null comment '上次对话模型ID',
    last_multi_model_id  bigint                             null comment '上次多模态模型ID',
    _tenant_id           bigint                             not null comment '租户ID',
    space_id             bigint                             null comment '空间ID',
    created              datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id           bigint                             null comment '创建人ID',
    creator_name         varchar(64)                        null comment '创建人',
    modified             datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id          bigint                             null comment '最后修改人ID',
    modified_name        varchar(64)                        null comment '最后修改人',
    yn                   tinyint  default 1                 not null comment '逻辑标记,1:有效;-1:无效',
    uid                  varchar(64)                        not null comment '唯一标识'
)
    comment '用户项目构建管理';

create index idx_dev_running_yn
    on custom_page_build (dev_running, yn);

create index idx_project_yn
    on custom_page_build (project_id, yn);

create table custom_page_config
(
    id              bigint auto_increment comment '主键ID'
        primary key,
    name            varchar(255)                            not null comment '项目名称',
    description     varchar(255)                            null comment '项目描述',
    icon            varchar(500)                            null comment '项目图标',
    base_path       varchar(255)                            not null comment '项目基础路径',
    build_running   tinyint                                 not null comment '线上运行标记,1:运行中;-1:未运行',
    publish_type    enum ('AGENT', 'PAGE')                  null,
    need_login      tinyint                                 null comment '是否需要登陆,1:需要',
    dev_agent_id    bigint                                  null comment '开发关联智能体ID',
    project_type    enum ('ONLINE_DEPLOY', 'REVERSE_PROXY') not null comment '项目类型',
    proxy_config    json                                    null comment '代理配置',
    page_arg_config json                                    null comment '路径参数配置',
    data_sources    json                                    null comment '绑定的数据源',
    ext             json                                    null comment '扩展参数',
    _tenant_id      bigint                                  not null comment '租户ID',
    space_id        bigint                                  null comment '空间ID',
    created         datetime default CURRENT_TIMESTAMP      not null comment '创建时间',
    creator_id      bigint                                  null comment '创建人ID',
    creator_name    varchar(64)                             null comment '创建人',
    modified        datetime default CURRENT_TIMESTAMP      null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id     bigint                                  null comment '最后修改人ID',
    modified_name   varchar(64)                             null comment '最后修改人',
    yn              tinyint  default 1                      not null comment '逻辑标记,1:有效;-1:无效',
    constraint uk_base_path
        unique (base_path)
)
    comment '用户页面配置';

create table custom_page_conversation
(
    id            bigint auto_increment comment '主键ID'
        primary key,
    project_id    bigint                             not null comment '项目ID',
    topic         varchar(500)                       null comment '会话主题',
    content       longtext                           not null comment '会话内容',
    _tenant_id    bigint                             not null comment '租户ID',
    space_id      bigint                             null comment '空间ID',
    created       datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                             not null comment '创建者ID',
    creator_name  varchar(255)                       not null comment '创建者名称',
    modified      datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '修改时间',
    modified_id   bigint                             null comment '修改者ID',
    modified_name varchar(255)                       null comment '修改者名称',
    yn            int      default 1                 not null comment '是否有效 1:有效 -1:无效'
)
    comment '自定义页面会话记录表';

create index idx_project_yn_created
    on custom_page_conversation (project_id, yn, created);

create table custom_table_definition
(
    id                bigint auto_increment comment '主键ID'
        primary key,
    _tenant_id        bigint                             not null comment '租户ID',
    space_id          bigint                             not null comment '所属空间ID',
    icon              varchar(255)                       null comment '图标图片地址',
    table_name        varchar(64)                        not null comment '表名',
    table_description varchar(256)                       null comment '表描述',
    doris_database    varchar(64)                        not null comment 'Doris数据库名',
    doris_table       varchar(64)                        not null comment 'Doris表名',
    status            tinyint  default 1                 not null comment '状态：1-启用 -1-禁用',
    created           datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id        bigint                             null comment '创建人id',
    creator_name      varchar(64)                        null comment '创建人',
    modified          datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id       bigint                             null comment '最后修改人id',
    modified_name     varchar(64)                        null comment '最后修改人',
    yn                tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效'
)
    comment '自定义数据表定义';

create index idx_table_name
    on custom_table_definition (table_name);

create table eco_market_client_config
(
    id                bigint auto_increment comment '主键id'
        primary key,
    uid               varchar(128)                       not null comment '唯一ID,分布式唯一UUID',
    name              varchar(128)                       not null comment '名称',
    description       varchar(256)                       null comment '描述',
    data_type         tinyint  default 1                 not null comment '市场类型,默认插件,1:插件;2:模板;3:MCP',
    target_type       varchar(64)                        null comment '细分类型,比如: 插件,智能体,工作流',
    target_id         bigint                             null comment '具体目标的id,可以智能体,工作流,插件,还有mcp等',
    category_code     varchar(128)                       null comment '分类编码,商业服务等,通过接口获取',
    category_name     varchar(128)                       null comment '分类名称,商业服务等,通过接口获取',
    owned_flag        tinyint  default 0                 not null comment '是否我的分享,0:否(生态市场获取的);1:是(我的分享)',
    share_status      tinyint  default 1                 not null comment '分享状态,1:草稿;2:审核中;3:已发布;4:已下线;5:驳回',
    use_status        tinyint  default 2                 not null comment '使用状态,1:启用;2:禁用;',
    publish_time      datetime                           null comment '发布时间',
    offline_time      datetime                           null comment '下线时间',
    version_number    bigint   default 1                 not null comment '版本号,自增,发布一次增加1,初始值为1',
    author            varchar(256)                       null comment '作者信息',
    publish_doc       mediumtext                         null comment '发布文档',
    config_param_json json                               null comment '请求参数配置json',
    config_json       json                               null comment '配置json,存储插件的配置信息如果有其他额外的信息保存放这里',
    icon              varchar(255)                       null comment '图标图片地址',
    _tenant_id        bigint                             not null comment '租户ID',
    create_client_id  varchar(128)                       not null comment '创建者的客户端ID',
    created           datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id        bigint                             null comment '创建人id',
    creator_name      varchar(64)                        null comment '创建人',
    modified          datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id       bigint                             null comment '最后修改人id',
    modified_name     varchar(64)                        null comment '最后修改人',
    yn                tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效',
    approve_message   varchar(256)                       null comment '审批原因',
    tenant_enabled    tinyint  default 0                 null comment '是否租户自动启用插件,1:租户自动启用;0:非租户自动启用;默认:0',
    constraint uk_uid
        unique (uid, _tenant_id)
)
    comment '生态市场配置';

create table eco_market_client_publish_config
(
    id                bigint auto_increment comment '主键id'
        primary key,
    uid               varchar(128)                       not null comment '唯一ID,分布式唯一UUID',
    name              varchar(128)                       not null comment '名称',
    description       varchar(256)                       null comment '描述',
    data_type         tinyint  default 1                 not null comment '市场类型,默认插件,1:插件;2:模板;3:MCP',
    target_type       varchar(64)                        null comment '细分类型,比如: 插件,智能体,工作流',
    target_id         bigint                             null comment '具体目标的id,可以智能体,工作流,插件,还有mcp等',
    category_code     varchar(128)                       null comment '分类编码,商业服务等,通过接口获取',
    category_name     varchar(128)                       null comment '分类名称,商业服务等,通过接口获取',
    owned_flag        tinyint  default 0                 not null comment '是否我的分享,0:否(生态市场获取的);1:是(我的分享)',
    share_status      tinyint  default 1                 not null comment '分享状态,1:草稿;2:审核中;3:已发布;4:已下线;5:驳回',
    use_status        tinyint  default 1                 not null comment '使用状态,1:启用;2:禁用;',
    publish_time      datetime                           null comment '发布时间',
    offline_time      datetime                           null comment '下线时间',
    version_number    bigint   default 1                 not null comment '版本号,自增,发布一次增加1,初始值为1',
    author            varchar(256)                       null comment '作者信息',
    publish_doc       mediumtext                         null comment '发布文档',
    config_param_json json                               null comment '请求参数配置json',
    config_json       json                               null comment '配置json,存储插件的配置信息如果有其他额外的信息保存放这里',
    icon              varchar(255)                       null comment '图标图片地址',
    _tenant_id        bigint                             not null comment '租户ID',
    create_client_id  varchar(128)                       not null comment '创建者的客户端ID',
    created           datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id        bigint                             null comment '创建人id',
    creator_name      varchar(64)                        null comment '创建人',
    modified          datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id       bigint                             null comment '最后修改人id',
    modified_name     varchar(64)                        null comment '最后修改人',
    yn                tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效',
    approve_message   varchar(256)                       null comment '审批原因',
    tenant_enabled    tinyint  default 0                 null comment '是否租户自动启用插件,1:租户自动启用;0:非租户自动启用;默认:0',
    constraint uk_uid
        unique (uid, _tenant_id)
)
    comment '生态市场,客户端,已发布配置';

create table eco_market_client_secret
(
    id            bigint auto_increment comment '主键id'
        primary key,
    name          varchar(128)                       not null comment '名称',
    description   varchar(256)                       null comment '描述',
    client_id     varchar(128)                       not null comment '客户端ID,分布式唯一UUID',
    client_secret varchar(256)                       null comment '客户端密钥',
    _tenant_id    bigint                             not null comment '租户ID',
    created       datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                             null comment '创建人id',
    creator_name  varchar(64)                        null comment '创建人',
    modified      datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    yn            tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效',
    constraint uk_client_id
        unique (client_id) comment '客户端ID唯一索引'
)
    comment '生态市场,客户端端配置';

create table knowledge_config
(
    id                 bigint auto_increment comment '主键id'
        primary key,
    name               varchar(128)                                            not null comment '知识库名称',
    description        varchar(256)                                            null comment '知识库描述',
    pub_status         enum ('Waiting', 'Published') default 'Waiting'         not null,
    data_type          tinyint                       default 1                 not null comment '数据类型,默认文本,1:文本;2:表格',
    embedding_model_id int                                                     null comment '知识库的嵌入模型ID',
    chat_model_id      int                                                     null comment '知识库的生成Q&A模型ID',
    _tenant_id         bigint                                                  not null comment '租户ID',
    space_id           bigint                                                  not null comment '所属空间ID',
    created            datetime                      default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id         bigint                                                  null comment '创建人id',
    creator_name       varchar(64)                                             null comment '创建人',
    modified           datetime                      default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id        bigint                                                  null comment '最后修改人id',
    modified_name      varchar(64)                                             null comment '最后修改人',
    yn                 tinyint                       default 1                 null comment '逻辑标记,1:有效;-1:无效',
    icon               varchar(255)                                            null comment '图标图片地址',
    file_size          bigint                        default 0                 null comment '文件大小,单位字节byte',
    workflow_id        bigint                                                  null comment '工作流id,可选,已工作流的形式,来执行解析文档获取文本的任务'
)
    comment '知识库表';

create table knowledge_document
(
    id            bigint auto_increment comment '主键id'
        primary key,
    kb_id         bigint                               not null comment '文档所属知识库',
    name          varchar(128)                         not null comment '文档名称',
    doc_url       varchar(256)                         not null comment '文件URL',
    pub_status    enum ('Waiting', 'Published')        not null,
    has_qa        tinyint(1) default 0                 not null comment '是否已经生成Q&A',
    has_embedding tinyint(1) default 0                 not null comment '是否已经完成嵌入',
    segment       json                                 null comment '文档分段方式（需要记录分段方式，基于字符数量或换行，Q&A字段等）。如果为空，表示还没有进行分段',
    _tenant_id    bigint                               not null comment '租户ID',
    space_id      bigint                               not null comment '所属空间ID',
    created       datetime   default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                               null comment '创建人id',
    creator_name  varchar(64)                          null comment '创建人',
    modified      datetime   default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id   bigint                               null comment '最后修改人id',
    modified_name varchar(64)                          null comment '最后修改人',
    yn            tinyint    default 1                 null comment '逻辑标记,1:有效;-1:无效',
    file_content  longtext                             null comment '自定义文本内容,自定义添加会有',
    data_type     tinyint    default 1                 not null comment '文件类型,1:URL访问文件;2:自定义文本内容',
    file_size     bigint     default 0                 null comment '文件大小,单位字节byte'
)
    comment '知识库-原始文档表';

create index idx_id_kb_id_index
    on knowledge_document (space_id, kb_id);

create index idx_kb_id
    on knowledge_document (kb_id);

create table knowledge_qa_segment
(
    id            bigint auto_increment comment '主键id'
        primary key,
    doc_id        bigint                               not null comment '分段所属文档',
    raw_id        bigint                               null comment '所属原始分段ID,前端手动新增的没有归属分段内容',
    question      text                                 null comment '问题会进行嵌入（对分段的增删改会走大模型并调用向量数据库）',
    answer        text                                 null comment '答案会进行嵌入（对分段的增删改会走大模型并调用向量数据库）',
    kb_id         bigint                               not null comment '知识库ID',
    has_embedding tinyint(1) default 0                 not null comment '是否已经完成嵌入',
    _tenant_id    bigint                               not null comment '租户ID',
    space_id      bigint                               not null comment '所属空间ID',
    created       datetime   default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                               null comment '创建人id',
    creator_name  varchar(64)                          null comment '创建人',
    modified      datetime   default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id   bigint                               null comment '最后修改人id',
    modified_name varchar(64)                          null comment '最后修改人',
    yn            tinyint    default 1                 null comment '逻辑标记,1:有效;-1:无效'
)
    comment '问答表';

create index idx_doc_id
    on knowledge_qa_segment (doc_id);

create index idx_kb_id_space_id_doc_id_index
    on knowledge_qa_segment (kb_id, space_id, doc_id);

create table knowledge_raw_segment
(
    id            bigint auto_increment comment '主键id'
        primary key,
    doc_id        bigint                             not null comment '分段所属文档',
    raw_txt       mediumtext                         null comment '原始文本',
    kb_id         bigint                             not null comment '知识库ID',
    sort_index    int                                not null comment '排序索引,在归属同一个文档下，段的排序',
    _tenant_id    bigint                             not null comment '租户ID',
    space_id      bigint                             not null comment '所属空间ID',
    created       datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                             null comment '创建人id',
    creator_name  varchar(64)                        null comment '创建人',
    modified      datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modified_id   bigint                             null comment '最后修改人id',
    modified_name varchar(64)                        null comment '最后修改人',
    yn            tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效',
    qa_status     tinyint  default -1                null comment '-1:待生成问答;1:已生成问答;'
)
    comment '原始分段（也称chunk）表，这些信息待生成问答后可以不再保存';

create index idx_doc_id
    on knowledge_raw_segment (doc_id);

create index idx_kb_id
    on knowledge_raw_segment (kb_id);

create index idx_space_id_kb_id_doc_id_index
    on knowledge_raw_segment (space_id, kb_id, doc_id);

create table knowledge_task
(
    id            bigint auto_increment comment '主键id'
        primary key,
    kb_id         bigint                             not null comment '文档所属知识库',
    space_id      bigint                             not null comment '所属空间ID',
    doc_id        bigint                             not null comment '文档id',
    type          tinyint                            not null comment '任务重试阶段类型:1:文档分段;2:生成Q&A;3:生成嵌入;10:任务完毕',
    tid           varchar(100)                       not null comment 'tid',
    name          varchar(128)                       not null comment '任务名称',
    status        tinyint                            not null comment '状态，0:初始状态,1待重试，2重试成功，3重试失败，4禁止重试',
    max_retry_cnt int      default 5                 not null comment '最大重试次数',
    retry_cnt     int      default 0                 not null comment '已重试次数',
    result        mediumtext                         null comment '调用结果',
    _tenant_id    bigint                             not null comment '租户ID',
    created       datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                             null comment '创建人id',
    creator_name  varchar(64)                        null comment '创建人',
    modified      datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    yn            tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效'
)
    comment '知识库-定时任务';

create index idx_doc_id
    on knowledge_task (doc_id);

create index idx_status_kb_id_doc_id
    on knowledge_task (status, kb_id, doc_id);

create table knowledge_task_history
(
    id            bigint auto_increment comment '主键id'
        primary key,
    kb_id         bigint                             not null comment '文档所属知识库',
    space_id      bigint                             not null comment '所属空间ID',
    doc_id        bigint                             not null comment '文档id',
    type          tinyint                            not null comment '任务重试阶段类型:1:文档分段;2:生成Q&A;3:生成嵌入;10:任务完毕',
    tid           varchar(100)                       not null comment 'tid',
    name          varchar(128)                       not null comment '任务名称',
    status        tinyint                            not null comment '状态，0:初始状态,1待重试，2重试成功，3重试失败，4禁止重试',
    max_retry_cnt int      default 5                 not null comment '最大重试次数',
    retry_cnt     int      default 0                 not null comment '已重试次数',
    result        mediumtext                         null comment '调用结果',
    _tenant_id    bigint                             not null comment '租户ID',
    created       datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    creator_id    bigint                             null comment '创建人id',
    creator_name  varchar(64)                        null comment '创建人',
    modified      datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    yn            tinyint  default 1                 null comment '逻辑标记,1:有效;-1:无效'
)
    comment '知识库-定时任务-历史';

create index idx_status_kb_id_doc_id
    on knowledge_task_history (status, kb_id, doc_id);

create table mcp_config
(
    id              bigint auto_increment
        primary key,
    _tenant_id      bigint      default 1                 not null comment '租户ID',
    space_id        bigint                                not null comment '空间ID',
    creator_id      bigint                                not null comment '创建用户ID',
    uid             varchar(64)                           null,
    name            varchar(64)                           not null comment 'MCP名称',
    server_name     varchar(64)                           null,
    description     text                                  null comment 'MCP描述信息',
    icon            varchar(255)                          null comment 'icon图片地址',
    category        varchar(64)                           null,
    install_type    varchar(64)                           not null comment 'MCP安装类型',
    deploy_status   varchar(64) default 'Initialization'  not null comment '部署状态',
    config          json                                  null comment 'MCP配置',
    deployed_config json                                  null comment 'MCP已发布的配置',
    deployed        datetime                              null comment '部署时间',
    modified        datetime    default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created         datetime    default CURRENT_TIMESTAMP not null
);

create index idx_uid
    on mcp_config (uid);

create table model_config
(
    id              bigint auto_increment
        primary key,
    _tenant_id      bigint                                                                                                              default 1                    not null comment '商户ID',
    space_id        bigint                                                                                                                                           null comment '空间ID',
    creator_id      bigint                                                                                                                                           not null comment '创建者ID',
    scope           enum ('Space', 'Tenant', 'Global')                                                                                  default 'Tenant'             not null comment '模型生效范围',
    name            varchar(255)                                                                                                                                     not null comment '模型名称',
    description     text                                                                                                                                             null comment '模型描述',
    model           varchar(128)                                                                                                                                     null comment '模型标识',
    type            varchar(64)                                                                                                                                      null comment '模型类型',
    is_reason_model int(4)                                                                                                              default 0                    not null comment '是否为深度思考模型',
    network_type    enum ('Internet', 'Intranet')                                                                                       default 'Internet'           not null comment '联网类型',
    nat_info        json                                                                                                                                             null comment '网络配置信息（内网模式使用）',
    function_call   enum ('Unsupported', 'CallSupported', 'StreamCallSupported')                                                        default 'CallSupported'      not null comment '函数调用支持程度',
    max_tokens      int(10)                                                                                                             default 4096                 not null comment '请求token上限',
    api_protocol    varchar(64)                                                                                                                                      not null comment '模型接口协议',
    api_info        json                                                                                                                                             not null comment 'API列表 [{"url":"","key":"","weight":1}]',
    strategy        enum ('RoundRobin', 'WeightedRoundRobin', 'LeastConnections', 'WeightedLeastConnections', 'Random', 'ResponseTime') default 'WeightedRoundRobin' not null comment '接口调用策略',
    dimension       int(10)                                                                                                             default 1536                 not null comment '向量维度',
    modified        datetime                                                                                                            default CURRENT_TIMESTAMP    not null on update CURRENT_TIMESTAMP comment '修改时间',
    created         datetime                                                                                                            default CURRENT_TIMESTAMP    not null comment '创建时间',
    enabled         tinyint                                                                                                                                          null comment '启用状态'
);

create table notify_message
(
    id         bigint auto_increment
        primary key,
    _tenant_id bigint   default 1                 not null comment '商户ID',
    sender_id  bigint                             null comment '发送用户',
    scope      varchar(32)                        not null comment '消息范围 Broadcast 广播消息；Private 私对私消息',
    content    mediumtext                         not null comment '消息内容',
    modified   datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created    datetime default CURRENT_TIMESTAMP not null
);

create table notify_message_user
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint                  default 1                 not null comment '商户ID',
    notify_id   bigint                                            null comment '通知消息ID',
    user_id     bigint                                            null comment '接收用户，广播消息user_id=-1',
    read_status enum ('Read', 'Unread') default 'Unread'          not null comment '已读状态',
    modified    datetime                default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created     datetime                default CURRENT_TIMESTAMP not null
);

create index idx_user_notify
    on notify_message_user (user_id, notify_id);

create table pf_retry_data
(
    id              bigint auto_increment
        primary key,
    project_code    varchar(50)                        not null comment '应用code',
    app_code        varchar(50)                        not null comment '模块code',
    bean_name       varchar(200)                       not null comment '服务接口',
    method_name     varchar(100)                       not null comment '接口方法',
    tid             varchar(100)                       not null comment 'tid',
    status          tinyint  default 1                 not null comment '状态，1待重试，2重试成功，3重试失败，4禁止重试',
    max_retry_cnt   int      default 5                 not null comment '最大重试次数',
    retry_cnt       int      default 0                 not null comment '已重试次数',
    arg_class_names varchar(600)                       null comment '参数类型名称组',
    arg_str         mediumtext                         null comment '参数数组JSONString格式，可编辑',
    result          mediumtext                         null comment '调用结果',
    creator_id      bigint                             null comment '操作人ID',
    creator_name    varchar(100)                       null comment '操作人名称',
    created         datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    modified        datetime default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间',
    modifier_id     bigint                             null comment '编辑人ID',
    modifier_name   varchar(100)                       null comment '编辑人名称',
    lock_time       datetime                           null comment '锁定至时间',
    yn              tinyint  default 1                 not null comment '是否有效',
    ext             longtext collate utf8mb4_bin       null comment '扩展信息',
    _tenant_id      bigint   default -1                not null
)
    comment '重试上报数据';

create index idx_project_app_bean_method
    on pf_retry_data (project_code, app_code, bean_name, method_name, tid);

create table plugin_config
(
    id             bigint auto_increment
        primary key,
    _tenant_id     bigint                                       default 1                 not null comment '租户ID',
    space_id       bigint                                                                 not null comment '空间ID',
    creator_id     bigint                                                                 not null comment '创建用户ID',
    name           varchar(64)                                                            not null comment '插件名称',
    description    text                                                                   null comment '插件描述信息',
    icon           varchar(255)                                                           null comment 'icon图片地址',
    type           varchar(64)                                                            not null comment '插件类型',
    code_lang      varchar(64)                                                            null comment '插件类型为代码时，该字段填写代码语言js、python',
    publish_status enum ('Developing', 'Applying', 'Published') default 'Developing'      not null comment '发布状态',
    config         json                                                                   null comment '插件配置',
    modified       datetime                                     default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created        datetime                                     default CURRENT_TIMESTAMP not null
);

create table publish_apply
(
    id             bigint auto_increment
        primary key,
    _tenant_id     bigint                                                       not null,
    space_id       bigint                                                       null comment '空间ID',
    apply_user_id  bigint                                                       not null comment '申请用户ID',
    target_type    enum ('Agent', 'Plugin', 'Workflow')                         not null comment '审核目标类型',
    target_id      bigint                                                       not null,
    name           varchar(64)                                                  not null comment '发布名称',
    description    text                                                         null comment '描述信息',
    icon           varchar(255)                                                 null comment '图标',
    remark         text                                                         null comment '发布记录',
    config         json                                                         not null comment '发布配置',
    channel        json                                                         null comment '发布渠道：Square 广场；System 系统发布',
    scope          enum ('Space', 'Tenant', 'Global') default 'Tenant'          null comment '发布范围',
    publish_status varchar(32)                                                  not null comment '发布审核状态',
    category       varchar(64)                        default ''                not null comment '分类',
    allow_copy     tinyint                            default 0                 not null comment '是否允许复制',
    only_template  tinyint                            default 0                 not null comment '仅展示模板',
    modified       datetime                           default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created        datetime                           default CURRENT_TIMESTAMP not null
);

create table published
(
    id              bigint auto_increment
        primary key,
    _tenant_id      bigint                                                       not null,
    space_id        bigint                                                       null comment '空间ID',
    user_id         bigint                                                       null comment '发布者ID',
    target_id       bigint                                                       not null comment '发布目标对象ID',
    target_type     enum ('Agent', 'Plugin', 'Workflow')                         not null comment '发布类型',
    target_sub_type varchar(32)                        default 'Single'          not null,
    name            varchar(64)                                                  not null comment '发布名称',
    description     text                                                         null comment '描述信息',
    icon            varchar(255)                                                 null comment '图标',
    remark          text                                                         null comment '发布记录',
    config          json                                                         not null comment '发布配置',
    channel         varchar(32)                        default 'Square'          not null comment '发布渠道：Square 广场；System 系统发布',
    scope           enum ('Tenant', 'Global', 'Space') default 'Tenant'          not null comment '发布范围',
    category        varchar(64)                        default 'Other'           null comment '分类',
    allow_copy      tinyint                            default 0                 not null comment '是否允许复制',
    only_template   tinyint                            default 0                 not null comment '仅展示模板',
    modified        datetime                           default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created         datetime                           default CURRENT_TIMESTAMP not null
)
    comment '发布';

create index target_id
    on published (target_id);

create table published_statistics
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint   default -1                  not null comment '商户ID',
    target_id   bigint                               not null comment '目标统计对象ID',
    target_type enum ('Agent', 'Plugin', 'Workflow') not null comment '目标对象类型',
    name        varchar(32)                          not null comment '统计名称',
    value       bigint   default 0                   not null comment '统计值',
    modified    datetime default CURRENT_TIMESTAMP   not null on update CURRENT_TIMESTAMP comment '更新时间',
    created     datetime default CURRENT_TIMESTAMP   not null comment '创建时间',
    constraint uk_target_id
        unique (target_id, target_type, name)
);

create table schedule_task
(
    id             bigint auto_increment
        primary key,
    task_id        varchar(255)                       not null comment '任务ID',
    bean_id        varchar(128)                       not null comment '回调处理器',
    cron           varchar(32)                        not null comment '执行周期',
    params         json                               null comment '附加参数',
    status         varchar(32)                        not null comment '调用状态',
    lock_time      datetime default CURRENT_TIMESTAMP not null comment '锁定时间',
    exec_times     bigint   default 0                 not null comment '已执行次数',
    max_exec_times bigint                             not null comment '最大执行次数',
    modified       datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created        datetime default CURRENT_TIMESTAMP not null comment '创建时间'
)
    comment '任务调度表';

create index idx_user_status
    on schedule_task (status);

create index status
    on schedule_task (status);

create index task_id
    on schedule_task (task_id);

create table space
(
    id              bigint auto_increment
        primary key,
    _tenant_id      bigint                             default 1                 not null comment '商户ID',
    name            varchar(64)                                                  not null comment '空间名称',
    description     varchar(255)                                                 null comment '空间介绍',
    icon            varchar(255)                                                 null comment '空间图标',
    creator_id      bigint                                                       not null comment '创建者ID',
    type            enum ('Personal', 'Team', 'Class') default 'Team'            not null comment '空间类型',
    receive_publish tinyint                            default 1                 not null comment '是否允许来自外部空间的发布',
    allow_develop   tinyint                            default 1                 not null comment '是否开启开发者功能',
    yn              tinyint                            default 0                 not null comment '逻辑删除，1为删除',
    modified        datetime                           default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '修改时间',
    created         datetime                           default CURRENT_TIMESTAMP not null comment '创建时间'
)
    comment '团队空间';

create index idx_tenant_id
    on space (_tenant_id);

create table space_user
(
    id         bigint auto_increment
        primary key,
    _tenant_id bigint                          default 1                 not null comment '商户ID',
    space_id   bigint                                                    not null comment '空间ID',
    user_id    bigint                                                    not null comment '人员ID',
    role       enum ('Owner', 'Admin', 'User') default 'User'            not null comment '空间角色',
    modified   datetime                        default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '修改时间',
    created    datetime                        default CURRENT_TIMESTAMP not null comment '创建时间',
    constraint uk_space_user
        unique (space_id, user_id)
)
    comment '空间人员';

create table sys_operator_log
(
    id              bigint auto_increment comment '自增主键'
        primary key,
    operate_type    tinyint                             not null comment '1:操作类型;2:访问日志',
    system_code     varchar(64)                         null comment '系统编码',
    system_name     varchar(64)                         not null comment '系统名称',
    object_op       varchar(64)                         not null comment '操作对象,比如:用户表,角色表,菜单表',
    action          varchar(64)                         not null comment '操作动作,比如:新增,删除,修改,查看',
    operate_content varchar(256)                        null comment '操作内容,比如评估页面',
    extra_content   text                                null comment '额外的操作内容信息记录,比如:更新提交的数据内容',
    org_id          bigint                              not null comment '操作人所属机构id',
    org_name        varchar(256)                        not null comment '操作人所属机构名称',
    creator_id      bigint                              not null comment '创建人id',
    creator         varchar(64)                         not null comment '创建人名称',
    created         timestamp default CURRENT_TIMESTAMP not null comment '创建时间',
    modified        timestamp default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '修改时间',
    _tenant_id      bigint                              not null comment '租户ID',
    yn              tinyint   default 1                 not null comment '是否有效；1：有效，-1：无效'
)
    comment '操作日志';

create table system_config
(
    config_id   int auto_increment
        primary key,
    name        varchar(32)                           not null,
    value       text                                  not null,
    type        varchar(32) default 'system'          not null,
    input_type  varchar(16)                           null,
    description varchar(255)                          not null,
    created     datetime    default CURRENT_TIMESTAMP not null,
    constraint name
        unique (name)
)
    comment '系统全局配置';

create table tenant
(
    id          bigint auto_increment
        primary key,
    name        varchar(255)                            not null comment '商户名称',
    description text                                    null comment '商户介绍',
    status      enum ('Pending', 'Enabled', 'Disabled') not null comment '商户状态',
    domain      varchar(64) default ''                  not null,
    modified    datetime    default CURRENT_TIMESTAMP   not null on update CURRENT_TIMESTAMP comment '更新时间',
    created     datetime    default CURRENT_TIMESTAMP   not null comment '创建时间',
    constraint uk_domain
        unique (domain)
);

create table tenant_config
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint                                not null,
    description varchar(255)                          not null,
    name        varchar(32)                           not null,
    value       json                                  not null,
    category    varchar(32) default 'Base'            null,
    input_type  varchar(16) default 'Input'           null,
    data_type   varchar(16) default 'String'          null,
    notice      varchar(255)                          not null,
    placeholder varchar(255)                          not null,
    min_height  int(50)                               null,
    required    varchar(8)  default 'true'            not null,
    sort        int(10)     default 0                 not null,
    created     datetime    default CURRENT_TIMESTAMP not null,
    constraint name
        unique (name, _tenant_id)
);

create table tool
(
    tool_id       bigint auto_increment
        primary key,
    tool_key      varchar(32)                            not null comment '工具唯一标识',
    name          varchar(64)                            not null comment '工具名称',
    icon_url      varchar(255)                           null comment '图标地址',
    description   text                                   null comment '工具描述',
    handler_clazz varchar(255)                           null comment '处理类',
    dto_clazz     varchar(255) default ''                not null comment 'DTO类',
    modified      datetime     default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created       datetime     default CURRENT_TIMESTAMP not null
);

create table user
(
    id              bigint auto_increment comment '平台用户ID'
        primary key,
    _tenant_id      bigint                       default 1                 not null comment '商户ID',
    uid             varchar(128)                                           null comment '用户唯一标识',
    user_name       varchar(64)                                            null comment '用户姓名',
    nick_name       varchar(64)                                            null comment '用户昵称',
    avatar          varchar(255)                                           null comment '用户头像',
    status          enum ('Enabled', 'Disabled') default 'Enabled'         not null comment '状态，启用或禁用',
    role            enum ('Admin', 'User')       default 'User'            null comment '角色',
    password        varchar(255)                                           not null comment '管理员密码',
    reset_pass      tinyint                      default 0                 not null comment '是否设置过密码',
    email           varchar(255)                                           null comment '管理员邮箱',
    phone           varchar(64)                                            null comment '电话号码',
    last_login_time datetime                                               null comment '最后登录时间',
    created         datetime                     default CURRENT_TIMESTAMP not null comment '创建时间',
    modified        datetime                     default CURRENT_TIMESTAMP null on update CURRENT_TIMESTAMP comment '更新时间'
)
    comment '用户表';

create index idx_phone
    on user (phone, _tenant_id);

create index idx_uid
    on user (uid, _tenant_id);

create table user_access_key
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint                             not null comment '租户ID',
    user_id     bigint                             not null comment '用户ID',
    target_type varchar(64)                        not null comment '目标业务类型',
    target_id   varchar(64)                        null comment '目标业务ID',
    access_key  varchar(255)                       null comment '访问密钥',
    config      json                               null comment '其他配置',
    modified    datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created     datetime default CURRENT_TIMESTAMP not null
);

create index idx_access_key
    on user_access_key (access_key);

create index idx_user_id
    on user_access_key (user_id);

create table user_agent_sort
(
    id                bigint auto_increment
        primary key,
    _tenant_id        bigint                             not null,
    user_id           bigint                             not null comment '用户ID',
    category          varchar(64)                        not null comment '排序分类',
    sort              int(10)                            not null comment '分类排序',
    agent_sort_config json                               null comment '分类下智能体排序配置',
    modified          datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created           datetime default CURRENT_TIMESTAMP not null,
    constraint uk_user_id_category
        unique (user_id, category)
);

create index idx_user_id
    on user_agent_sort (user_id);

create table user_target_relation
(
    id          bigint auto_increment
        primary key,
    _tenant_id  bigint                                not null comment '商户ID',
    user_id     bigint                                not null comment '用户ID',
    target_type varchar(32)                           not null comment '目标对象类型',
    target_id   bigint                                not null comment '目标对象ID',
    type        varchar(32) default 'Add'             not null comment '关系类型',
    modified    datetime    default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created     datetime    default CURRENT_TIMESTAMP not null comment '创建时间',
    constraint uk_user_agent_type
        unique (user_id, target_type, target_id, type)
)
    comment '用户与目标对象关系表';

create index idx_user_id
    on user_target_relation (user_id);

create table workflow_config
(
    id             bigint auto_increment
        primary key,
    _tenant_id     bigint                                       default 1                 not null comment '租户ID',
    space_id       bigint                                                                 not null comment '空间ID',
    creator_id     bigint                                                                 not null comment '创建用户ID',
    name           varchar(100)                                                           not null comment '工作流名称',
    description    text                                                                   null comment '工作流描述信息',
    icon           varchar(255)                                                           null comment 'icon图片地址',
    start_node_id  bigint                                                                 null comment '起始节点ID',
    end_node_id    bigint                                                                 null comment '结束节点ID',
    publish_status enum ('Developing', 'Applying', 'Published') default 'Developing'      not null comment '发布状态',
    ext            json                                                                   null,
    modified       datetime                                     default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created        datetime                                     default CURRENT_TIMESTAMP not null
);

create table workflow_node_config
(
    id                  bigint auto_increment
        primary key,
    _tenant_id          bigint   default 1                 not null comment '商户ID',
    name                varchar(100)                       null comment '节点名称',
    icon                varchar(255)                       null comment '图标',
    description         text                               null comment '描述',
    workflow_id         bigint                             null comment '工作流ID',
    type                varchar(32)                        not null comment '节点类型',
    config              json                               null comment '详细配置',
    loop_node_id        bigint                             null comment '循环体中各节点记录循环节点ID',
    next_node_ids       json                               null comment '下级节点ID列表',
    inner_node_Ids      json                               null comment '循环节点的内部节点',
    inner_start_node_id bigint                             null comment '循环节点内部开始节点',
    inner_end_node_id   bigint                             null comment '循环节点内部结束节点',
    modified            datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP,
    created             datetime default CURRENT_TIMESTAMP not null
)
    comment '智能体组件配置';


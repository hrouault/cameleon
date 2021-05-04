use super::{
    elem_type::ImmOrPNode,
    interface::IString,
    node_base::{NodeAttributeBase, NodeBase, NodeElementBase},
    store::{CacheStore, NodeStore, StringId, ValueStore},
    Device, GenApiError, GenApiResult, ValueCtxt,
};

#[derive(Debug, Clone)]
pub struct StringNode {
    pub(crate) attr_base: NodeAttributeBase,
    pub(crate) elem_base: NodeElementBase,

    pub(crate) streamable: bool,
    pub(crate) value: ImmOrPNode<StringId>,
}

impl StringNode {
    #[must_use]
    pub fn node_base(&self) -> NodeBase<'_> {
        NodeBase::new(&self.attr_base, &self.elem_base)
    }

    #[must_use]
    pub fn streamable(&self) -> bool {
        self.streamable
    }

    #[must_use]
    pub fn value_elem(&self) -> ImmOrPNode<StringId> {
        self.value
    }
}

impl IString for StringNode {
    #[tracing::instrument(skip(self, device, store, cx),
                          level = "trace",
                          fields(node = store.name_by_id(self.node_base().id()).unwrap()))]
    fn value<T: ValueStore, U: CacheStore>(
        &self,
        device: &mut impl Device,
        store: &impl NodeStore,
        cx: &mut ValueCtxt<T, U>,
    ) -> GenApiResult<String> {
        self.elem_base.verify_is_readable(device, store, cx)?;
        match self.value {
            ImmOrPNode::Imm(vid) => Ok(cx.value_store().str_value(vid).unwrap().clone()),
            ImmOrPNode::PNode(nid) => nid.expect_istring_kind(store)?.value(device, store, cx),
        }
    }

    #[tracing::instrument(skip(self, device, store, cx),
                          level = "trace",
                          fields(node = store.name_by_id(self.node_base().id()).unwrap()))]
    fn set_value<T: ValueStore, U: CacheStore>(
        &self,
        value: &str,
        device: &mut impl Device,
        store: &impl NodeStore,
        cx: &mut ValueCtxt<T, U>,
    ) -> GenApiResult<()> {
        self.elem_base.verify_is_writable(device, store, cx)?;
        if !value.is_ascii() {
            return Err(GenApiError::invalid_data(
                "the data to write must be an ascii string".into(),
            ));
        }
        if value.len() > self.max_length(device, store, cx)? as usize {
            return Err(GenApiError::invalid_data(
                "the data to write exceeds the maximum length allowed by the node.".into(),
            ));
        };
        cx.invalidate_cache_by(self.node_base().id());
        match self.value {
            ImmOrPNode::Imm(vid) => {
                cx.value_store_mut().update(vid, value.to_string());
                Ok(())
            }
            ImmOrPNode::PNode(nid) => nid
                .expect_istring_kind(store)?
                .set_value(value, device, store, cx),
        }
    }

    #[tracing::instrument(skip(self, device, store, cx),
                          level = "trace",
                          fields(node = store.name_by_id(self.node_base().id()).unwrap()))]
    fn max_length<T: ValueStore, U: CacheStore>(
        &self,
        device: &mut impl Device,
        store: &impl NodeStore,
        cx: &mut ValueCtxt<T, U>,
    ) -> GenApiResult<i64> {
        match self.value {
            ImmOrPNode::Imm(_) => Ok(i64::MAX),
            ImmOrPNode::PNode(nid) => nid
                .expect_istring_kind(store)?
                .max_length(device, store, cx),
        }
    }

    fn is_readable<T: ValueStore, U: CacheStore>(
        &self,
        device: &mut impl Device,
        store: &impl NodeStore,
        cx: &mut ValueCtxt<T, U>,
    ) -> GenApiResult<bool> {
        self.elem_base.is_readable(device, store, cx)
    }

    fn is_writable<T: ValueStore, U: CacheStore>(
        &self,
        device: &mut impl Device,
        store: &impl NodeStore,
        cx: &mut ValueCtxt<T, U>,
    ) -> GenApiResult<bool> {
        self.elem_base.is_writable(device, store, cx)
    }
}
//! ERC721 base contract.
//! The logic was based off of: https://github.com/cygaar/ERC721Stylus/blob/main/src/erc721.rs
//! Doc comments are forked from: https://github.com/Vectorized/solady/blob/main/src/tokens/ERC721.sol
//!
//! Implementation of the ERC-721 standard updated to use Stylus SDK version 0.5+
//! This contract use as base inkmate from his 0.0.6 version
//! https://github.com/cygaar/inkmate/blob/main/contracts/src/tokens/erc721.rs
//!

extern crate alloc;

use alloc::{string::String, vec, vec::Vec};
use core::{borrow::BorrowMut, marker::PhantomData};
use stylus_sdk::{
    abi::Bytes,
    alloy_primitives::{Address, U256},
    alloy_sol_types::sol,
    evm, msg,
    prelude::*,
};

pub trait ERC721Params {
    const NAME: &'static str;
    const SYMBOL: &'static str;
    fn token_uri(id: U256) -> String;
}

sol_storage! {
    /// ERC721 implements all ERC-721 methods
    pub struct ERC721<T: ERC721Params> {
        /// Maps id to owner
        mapping(uint256 => address) owners;
        /// Maps id to the approved spender
        mapping(uint256 => address) approved;
        /// Maps owner to their NFT balance
        mapping(address => uint256) balance;
        /// Maps the approved spenders for a given address
        mapping(address => mapping(address => bool)) approved_for_all;
        PhantomData<T> phantom;
    }
}

// Declare events and Solidity error types
sol! {
    /// Emitted when token `id` is transferred from `from` to `to`.
    event Transfer(address indexed from, address indexed to, uint256 indexed id);
    /// Emitted when `owner` enables `account` to manage the `id` token.
    event Approval(address indexed owner, address indexed approved, uint256 indexed id);
    /// Emitted when `owner` enables or disables `operator` to manage all of their tokens.
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);

    /// Token already minted.
    error AlreadyMinted();
    /// Invalid token id.
    error InvalidTokenId(uint256 id);
    /// Not the owner of the token.
    error NotOwner(address from, uint256 id, address real_owner);
    /// Not approved to transfer the token.
    error NotApproved(uint256 id, address owner, address spender);
    /// Transfer to the zero address no allowed.
    error TransferToZero(uint256 id);
    /// Safe transfer callback failed.
    error ReceiverRefused(address receiver, uint256 id);
}

#[derive(SolidityError)]
pub enum ERC721Error {
    AlreadyMinted(AlreadyMinted),
    InvalidTokenId(InvalidTokenId),
    NotOwner(NotOwner),
    NotApproved(NotApproved),
    TransferToZero(TransferToZero),
    ReceiverRefused(ReceiverRefused),
}

impl<T: ERC721Params> ERC721<T> {
    /// Requires that msg::sender() is authorized to spend a given token
    fn _require_authorized_to_spend(&self, from: Address, id: U256) -> Result<(), ERC721Error> {
        let owner = self.owner_of(id)?;
        if from != owner {
            return Err(ERC721Error::NotOwner(NotOwner {
                from,
                id,
                real_owner: owner,
            }));
        }

        if msg::sender() == owner {
            return Ok(());
        }
        if self.approved_for_all.getter(owner).get(msg::sender()) {
            return Ok(());
        }
        if msg::sender() == self.approved.get(id) {
            return Ok(());
        }
        Err(ERC721Error::NotApproved(NotApproved {
            owner,
            spender: msg::sender(),
            id,
        }))
    }

    /// Internal transfer function
    pub fn _transfer(&mut self, id: U256, from: Address, to: Address) -> Result<(), ERC721Error> {
        let mut owner = self.owners.setter(id);
        let previous_owner = owner.get();
        if previous_owner != from {
            return Err(ERC721Error::NotOwner(NotOwner {
                from,
                id,
                real_owner: previous_owner,
            }));
        }
        owner.set(to);

        // right now working with storage can be verbose, but this will change upcoming version of the Stylus SDK
        let mut from_balance = self.balance.setter(from);
        let balance = from_balance.get() - U256::from(1);
        from_balance.set(balance);

        let mut to_balance = self.balance.setter(to);
        let balance = to_balance.get() + U256::from(1);
        to_balance.set(balance);

        self.approved.delete(id);
        evm::log(Transfer { from, to, id });
        Ok(())
    }

    /// Calls the onERC721Received callback function if the receiver is not an EOA (code size > 0).
    /// Throws an error if the receiver cannot be called or the returned value is not ERC721_RECEIVED_SELECTOR.
    fn _call_receiver<S: TopLevelStorage>(
        storage: &mut S,
        id: U256,
        from: Address,
        to: Address,
        data: Vec<u8>,
    ) -> Result<(), ERC721Error> {
        if to.has_code() {
            let receiver = IERC721TokenReceiver::new(to);
            let received = receiver
                .on_erc_721_received(storage, msg::sender(), from, id, data.into())
                .map_err(|_| {
                    ERC721Error::ReceiverRefused(ReceiverRefused {
                        receiver: receiver.address,
                        id,
                    })
                })?
                .0;

            if u32::from_be_bytes(received) != ERC721_RECEIVED_SELECTOR {
                return Err(ERC721Error::ReceiverRefused(ReceiverRefused {
                    receiver: receiver.address,
                    id,
                }));
            }
        }
        Ok(())
    }

    /// Transfers token `id` from `from` to `to`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must exist.
    /// - `from` must be the owner of the token.
    /// - `to` cannot be the zero address.
    /// - The caller must be the owner of the token, or be approved to manage the token.
    /// - If `to` refers to a smart contract, it must implement
    ///   {IERC721Receiver-onERC721Received}, which is called upon a safe transfer.
    ///
    /// Emits a {Transfer} event.
    pub fn _safe_transfer<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        id: U256,
        from: Address,
        to: Address,
        data: Vec<u8>,
    ) -> Result<(), ERC721Error> {
        storage.borrow_mut().transfer_from(from, to, id)?;
        Self::_call_receiver(storage, id, from, to, data)
    }

    /// Mints token `id` to `to`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must not exist.
    /// - `to` cannot be the zero address.
    ///
    /// Emits a {Transfer} event.
    pub fn _mint(&mut self, to: Address, id: U256) -> Result<(), ERC721Error> {
        if to.is_zero() {
            return Err(ERC721Error::TransferToZero(TransferToZero { id }));
        }
        let mut owner = self.owners.setter(id);
        if !owner.is_zero() {
            return Err(ERC721Error::AlreadyMinted(AlreadyMinted {}));
        }
        owner.set(to);

        let mut to_balance = self.balance.setter(to);
        let balance = to_balance.get() + U256::from(1);
        to_balance.set(balance);

        evm::log(Transfer {
            from: Address::default(),
            to,
            id,
        });
        Ok(())
    }

    /// Mints token `id` to `to`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must not exist.
    /// - `to` cannot be the zero address.
    /// - If `to` refers to a smart contract, it must implement
    ///   {IERC721Receiver-onERC721Received}, which is called upon a safe transfer.
    ///
    /// Emits a {Transfer} event.
    pub fn _safe_mint<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        to: Address,
        id: U256,
        data: Vec<u8>,
    ) -> Result<(), ERC721Error> {
        storage.borrow_mut()._mint(to, id)?;
        Self::_call_receiver(storage, id, Address::default(), to, data)?;
        Ok(())
    }

    /// Destroys token `id`, using `by`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must exist.
    /// - If `by` is not the zero address,
    ///   it must be the owner of the token, or be approved to manage the token.
    ///
    /// Emits a {Transfer} event.
    pub fn _burn(&mut self, id: U256) -> Result<(), ERC721Error> {
        let mut owner_setter = self.owners.setter(id);
        if owner_setter.is_zero() {
            return Err(ERC721Error::InvalidTokenId(InvalidTokenId { id }));
        }
        let owner = owner_setter.get();

        if msg::sender() != owner
            && !self.approved_for_all.getter(owner).get(msg::sender())
            && msg::sender() != self.approved.get(id)
        {
            return Err(ERC721Error::NotApproved(NotApproved {
                owner,
                spender: msg::sender(),
                id,
            }));
        }

        let mut owner_balance = self.balance.setter(owner);
        let balance = owner_balance.get() - U256::from(1);
        owner_balance.set(balance);

        owner_setter.set(Address::default());
        self.approved.delete(id);

        evm::log(Transfer {
            from: owner,
            to: Address::default(),
            id,
        });
        Ok(())
    }
}

sol_interface! {
    /// Allows calls to the `onERC721Received` method of other contracts implementing `IERC721TokenReceiver`.
    interface IERC721TokenReceiver {
        function onERC721Received(address operator, address from, uint256 id, bytes data) external returns(bytes4);
    }
}

/// Selector for `onERC721Received`, which is returned by contracts implementing `IERC721TokenReceiver`.
const ERC721_RECEIVED_SELECTOR: u32 = 0x150b7a02;

#[external]
impl<T: ERC721Params> ERC721<T> {
    /// Returns the token collection name.
    pub fn name() -> String {
        T::NAME.into()
    }

    /// Returns the token collection symbol.
    pub fn symbol() -> String {
        T::SYMBOL.into()
    }

    /// Returns the Uniform Resource Identifier (URI) for token `id`.
    #[selector(name = "tokenURI")]
    pub fn token_uri(&self, id: U256) -> Result<String, ERC721Error> {
        self.owner_of(id)?; // require NFT exist
        Ok(T::token_uri(id))
    }

    /// Returns true if this contract implements the interface defined by `interfaceId`.
    /// See: https://eips.ethereum.org/EIPS/eip-165
    pub fn supports_interface(interface: [u8; 4]) -> bool {
        if interface == [0xff; 4] {
            // special cased in the ERC165 standard
            return false;
        }

        const IERC165: u32 = 0x01ffc9a7;
        const IERC721: u32 = 0x80ac58cd;
        const IERC721METADATA: u32 = 0x5b5e139f;

        matches!(
            u32::from_be_bytes(interface),
            IERC165 | IERC721 | IERC721METADATA
        )
    }

    /// Returns the number of tokens owned by `owner`.
    ///
    /// Requirements:
    /// - `owner` must not be the zero address.
    pub fn balance_of(&self, owner: Address) -> U256 {
        U256::from(self.balance.get(owner))
    }

    /// Returns the owner of token `id`.
    ///
    /// Requirements:
    /// - Token `id` must exist.
    pub fn owner_of(&self, id: U256) -> Result<Address, ERC721Error> {
        let owner = self.owners.get(id);
        if owner.is_zero() {
            return Err(ERC721Error::InvalidTokenId(InvalidTokenId { id }));
        }
        Ok(owner)
    }

    /// Transfers token `id` from `from` to `to`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must exist.
    /// - `from` must be the owner of the token.
    /// - `to` cannot be the zero address.
    /// - The caller must be the owner of the token, or be approved to manage the token.
    /// - If `to` refers to a smart contract, it must implement
    ///   {IERC721Receiver-onERC721Received}, which is called upon a safe transfer.
    ///
    /// Emits a {Transfer} event.
    pub fn safe_transfer_from<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        from: Address,
        to: Address,
        id: U256,
    ) -> Result<(), ERC721Error> {
        Self::safe_transfer_from_with_data(storage, from, to, id, Bytes(vec![]))
    }

    /// Equivalent to [`safe_transfer_from`], but with additional data for the receiver.
    ///
    /// Note: because Rust doesn't allow multiple methods with the same name,
    /// we use the `#[selector]` macro attribute to simulate solidity overloading.
    #[selector(name = "safeTransferFrom")]
    pub fn safe_transfer_from_with_data<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        from: Address,
        to: Address,
        id: U256,
        data: Bytes,
    ) -> Result<(), ERC721Error> {
        if to.is_zero() {
            return Err(ERC721Error::TransferToZero(TransferToZero { id }));
        }
        storage
            .borrow_mut()
            ._require_authorized_to_spend(from, id)?;

        Self::_safe_transfer(storage, id, from, to, data.0)
    }

    /// Transfers token `id` from `from` to `to`.
    ///
    /// Requirements:
    ///
    /// - Token `id` must exist.
    /// - `from` must be the owner of the token.
    /// - `to` cannot be the zero address.
    /// - The caller must be the owner of the token, or be approved to manage the token.
    ///
    /// Emits a {Transfer} event.
    pub fn transfer_from(
        &mut self,
        from: Address,
        to: Address,
        id: U256,
    ) -> Result<(), ERC721Error> {
        if to.is_zero() {
            return Err(ERC721Error::TransferToZero(TransferToZero { id }));
        }
        self._require_authorized_to_spend(from, id)?;
        self._transfer(id, from, to)?;
        Ok(())
    }

    /// Sets `account` as the approved account to manage token `id`.
    ///
    /// Requirements:
    /// - Token `id` must exist.
    /// - The caller must be the owner of the token,
    ///   or an approved operator for the token owner.
    ///
    /// Emits an {Approval} event.
    pub fn approve(&mut self, approved: Address, id: U256) -> Result<(), ERC721Error> {
        let owner = self.owner_of(id)?;

        // require authorization
        if msg::sender() != owner && !self.approved_for_all.getter(owner).get(msg::sender()) {
            return Err(ERC721Error::NotApproved(NotApproved {
                owner,
                spender: msg::sender(),
                id,
            }));
        }
        self.approved.insert(id, approved);

        evm::log(Approval {
            approved,
            owner,
            id,
        });
        Ok(())
    }

    /// Sets whether `operator` is approved to manage the tokens of the caller.
    ///
    /// Emits an {ApprovalForAll} event.
    pub fn set_approval_for_all(&mut self, operator: Address, approved: bool) {
        let owner = msg::sender();
        self.approved_for_all
            .setter(owner)
            .insert(operator, approved);

        evm::log(ApprovalForAll {
            owner,
            operator,
            approved,
        });
    }

    /// Returns the account approved to manage token `id`.
    /// Returns the zero address instead of reverting if the token does not exist.
    pub fn get_approved(&mut self, id: U256) -> Address {
        self.approved.get(id)
    }

    /// Returns whether `operator` is approved to manage the tokens of `owner`.
    pub fn is_approved_for_all(&mut self, owner: Address, operator: Address) -> bool {
        self.approved_for_all.getter(owner).get(operator)
    }
}
